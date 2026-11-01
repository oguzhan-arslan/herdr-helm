//! The system kind: CPU and memory of the machine, and the apps that use the most. Built in:
//! macos and linux (the default follows the platform). Others: an external program through the
//! command form (ops "usage", "apps").

mod linux;
mod macos;

use anyhow::Result;
use serde_json::Value;

pub trait System {
    /// The connector's name, e.g. "macos".
    fn name(&self) -> String;
    /// What it can do: "usage", "apps".
    fn capabilities(&self) -> Vec<String>;
    /// {"cpu": percent in use, "mem": percent in use, "total": memory in bytes}.
    fn usage(&self) -> Result<Value>;
    /// The processes, grouped by app: [{"name", "cpu" (percent of one core), "kb" (resident)}],
    /// and the machine: {"cores", "total" (bytes)}. Helm ranks them.
    fn apps(&self) -> Result<(Vec<App>, u64, u64)>;
}

pub struct App {
    pub name: String,
    pub cpu: f64,
    pub kb: f64,
    pub count: u64,
}

fn default_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// The chosen system connector: projects.json "connectors.system" (default by platform).
pub fn current() -> Result<Box<dyn System>> {
    match super::choice("system", default_name()) {
        super::Choice::Command(program) => Ok(Box::new(super::command::Command::new("system", program))),
        super::Choice::Builtin(name) => match name.as_str() {
            "macos" => Ok(Box::new(macos::MacOs)),
            "linux" => Ok(Box::new(linux::Linux)),
            "none" => anyhow::bail!("the system connector is off (connectors.system is \"none\")"),
            other => anyhow::bail!("unknown system connector '{other}' (built in: macos, linux)"),
        },
    }
}

/// The chosen system connector, when it can do `capability`.
pub fn current_with(capability: &str) -> Result<Box<dyn System>> {
    let sys = current()?;
    if !super::has(&sys.capabilities(), capability) {
        anyhow::bail!("the system connector '{}' cannot do '{capability}'", sys.name());
    }
    Ok(sys)
}

pub fn describe() -> Value {
    match current() {
        Ok(s) => serde_json::json!({ "name": s.name(), "capabilities": s.capabilities() }),
        Err(_) => super::none(),
    }
}

/// `ps -Aeo pcpu=,rss=,comm=` grouped by app: a helper inside an app bundle counts for the
/// outermost .app (Chrome's or VS Code's helpers are one row).
pub fn apps_of(text: &str) -> Vec<App> {
    let mut apps: Vec<App> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(cpu), Some(kb)) = (parts.next(), parts.next()) else { continue };
        let (Ok(cpu), Ok(kb)) = (cpu.parse::<f64>(), kb.parse::<f64>()) else { continue };
        let path = parts.collect::<Vec<_>>().join(" ");
        if path.is_empty() {
            continue;
        }
        let name = match path.find(".app/").or_else(|| path.ends_with(".app").then(|| path.len() - 4)) {
            Some(end) => path[..end].rsplit('/').next().unwrap_or(&path).to_string(),
            None => path.rsplit('/').next().unwrap_or(&path).to_string(),
        };
        match apps.iter_mut().find(|a| a.name == name) {
            Some(a) => {
                a.cpu += cpu;
                a.kb += kb;
                a.count += 1;
            }
            None => apps.push(App { name, cpu, kb, count: 1 }),
        }
    }
    apps
}

/// The apps that weigh most: impact is the share of all cores plus the share of all memory.
pub fn ranked(apps: Vec<App>, cores: u64, total: u64, top: usize) -> Value {
    let (cores, total) = (cores.max(1), total.max(1));
    let mut rows: Vec<Value> = apps
        .into_iter()
        .map(|a| {
            let impact = a.cpu / cores as f64 + a.kb * 1024.0 / total as f64 * 100.0;
            serde_json::json!({ "name": a.name, "cpu": a.cpu, "kb": a.kb, "count": a.count, "impact": impact })
        })
        .collect();
    rows.sort_by(|x, y| y["impact"].as_f64().unwrap_or(0.0).total_cmp(&x["impact"].as_f64().unwrap_or(0.0)));
    rows.truncate(top);
    serde_json::json!({ "cores": cores, "total": total, "apps": rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_helpers_under_their_app() {
        let text = " 10.0 1000 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome\n 5.5 500 /Applications/Google Chrome.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper\n 1.0 10 /usr/sbin/cfprefsd\nnot a line";
        let apps = apps_of(text);
        assert_eq!(apps.len(), 2);
        let chrome = apps.iter().find(|a| a.name == "Google Chrome").expect("chrome");
        assert_eq!(chrome.count, 2);
        assert!((chrome.cpu - 15.5).abs() < 1e-9);
        assert!(apps.iter().any(|a| a.name == "cfprefsd"));
    }

    #[test]
    fn ranks_by_impact() {
        let apps = vec![App { name: "a".into(), cpu: 100.0, kb: 0.0, count: 1 }, App { name: "b".into(), cpu: 0.0, kb: 0.0, count: 1 }];
        let r = ranked(apps, 4, 1 << 30, 1);
        assert_eq!(r["apps"].as_array().map(Vec::len), Some(1));
        assert_eq!(r["apps"][0]["name"], "a");
    }
}
