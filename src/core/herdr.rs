//! Herdr and hc: commands to the Herdr window (`herdr-app do`) and to herdr-core (`hc`). Herdr
//! is the host of Helm, not an external service, so these are not a connector.

use crate::util::{exec, run, tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::Path;

/// `herdr-app do <command> <args>`: the reply as JSON ({} when it is not JSON).
pub fn herdr(command: &str, args: &Value) -> Result<Value> {
    let text = run(&tool("herdr-app"), &["do", command, &args.to_string()], None)?;
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({})))
}

/// `hc <args>`: trimmed stdout; fails on a non-zero exit.
pub fn hc(args: &[&str]) -> Result<String> {
    run(&tool("hc"), args, None)
}

/// `hc <args>` that may fail: None on a non-zero exit.
pub fn hc_maybe(args: &[&str]) -> Option<String> {
    let out = exec(&tool("hc"), args, None, None).ok()?;
    out.ok().then(|| out.stdout.trim().to_string())
}

/// Every live terminal with its labels and parent, by address (`hc get` of each).
pub fn terminals() -> Result<Map<String, Value>> {
    let list: Value = serde_json::from_str(&hc(&["ls", "--json"]).unwrap_or_else(|_| "{}".into())).unwrap_or_else(|_| json!({}));
    let mut found = Map::new();
    for t in list["terminals"].as_array().cloned().unwrap_or_default() {
        let Some(addr) = t["address"].as_str() else { continue };
        if let Some(text) = hc_maybe(&["get", addr, "--json"]) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                found.insert(addr.to_string(), v);
            }
        }
    }
    Ok(found)
}

/// The Herdr space a terminal is in: its own label, else its parent's.
pub fn herdr_space_of(addr: Option<&str>, terms: &Map<String, Value>) -> Option<String> {
    let mut seen = HashSet::new();
    let mut addr = addr.map(String::from);
    while let Some(a) = addr {
        if !seen.insert(a.clone()) {
            break;
        }
        let t = terms.get(&a)?;
        if let Some(label) = t["labels"]["herdr.space"].as_str() {
            return Some(label.to_string());
        }
        addr = t["parent"].as_str().map(String::from);
    }
    None
}

/// The Herdr space of a space record: its own field, else a terminal labeled with a space that
/// works in the space's folder.
pub fn herdr_space_id(rec: &Value, space_dir: &Path, terms: &Map<String, Value>) -> Option<String> {
    if let Some(id) = rec["herdr_space"].as_str() {
        return Some(id.to_string());
    }
    // A path compare, not a text compare: tasks/foo must not match tasks/foo-2.
    terms.values().find_map(|t| {
        let label = t["labels"]["herdr.space"].as_str()?;
        t["cwd"].as_str().filter(|cwd| Path::new(cwd).starts_with(space_dir)).map(|_| label.to_string())
    })
}

/// The agents in a space: (its Herdr space id, a row per agent terminal).
pub fn space_agents(rec: &Value, space_dir: &Path) -> Result<(Option<String>, Vec<Value>)> {
    let terms = terminals()?;
    let sid = herdr_space_id(rec, space_dir, &terms);
    let me = std::env::var("HC_TERMINAL_ID").ok();
    let mut found = Vec::new();
    for (addr, t) in &terms {
        if sid.is_none() || herdr_space_of(Some(addr), &terms) != sid {
            continue;
        }
        let status = &t["status"];
        let Some(app) = status["app"].as_str() else { continue };
        let cwd = t["cwd"].as_str().unwrap_or("");
        let place = place_in(cwd, space_dir);
        let role = t["labels"]["herdr.role"].as_str().map(String::from).unwrap_or_else(|| {
            if t["parent"].is_null() {
                "main".into()
            } else {
                "subagent".into()
            }
        });
        found.push(json!({
            "address": addr, "name": t["name"], "agent": app, "status": status["state"],
            "role": role, "parent": t["parent"], "where": place, "you": Some(addr.as_str()) == me.as_deref(),
        }));
    }
    Ok((sid, found))
}

/// Where a terminal works, relative to the space folder: "." for the folder itself, else the
/// path inside it, else the full path when it is outside.
fn place_in(cwd: &str, space_dir: &Path) -> String {
    match Path::new(cwd).strip_prefix(space_dir) {
        Ok(rest) if rest.as_os_str().is_empty() => ".".to_string(),
        Ok(rest) => rest.to_string_lossy().into_owned(),
        Err(_) => cwd.to_string(),
    }
}

/// Agents run in hc terminals. Only the Herdr app (no terminal) completes or deletes a space.
/// No setting turns this off. The tests call the commands without this check.
pub fn refuse_in_terminal(what: &str) -> Result<()> {
    if std::env::var_os("HC_TERMINAL_ID").is_some() {
        return Err(anyhow!("'{what}' runs only from Herdr: use the buttons of the space. Agents may not run it."));
    }
    Ok(())
}

/// Ends every terminal of a space and closes its Herdr space. `complete` and `delete` call it
/// only after the space is removed, so a failed removal leaves the agents running.
pub fn close_herdr_space(rec: &Value, space_dir: &Path) -> Result<()> {
    let terms = terminals()?;
    let Some(sid) = herdr_space_id(rec, space_dir, &terms) else { return Ok(()) };
    // Handoff agents outlive their parent, so space.close leaves them running, and their labels
    // would bring the space back. End every terminal of the space first.
    for addr in terms.keys() {
        if herdr_space_of(Some(addr), &terms).as_deref() == Some(sid.as_str()) {
            let _ = hc_maybe(&["kill", addr]);
        }
    }
    // With its last terminal gone, Herdr may have closed the space already.
    match herdr("space.close", &json!({ "id": sid, "confirmed": true })) {
        Err(e) if !e.to_string().contains("no space") => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_of_follows_parents() {
        let terms: Map<String, Value> = serde_json::from_value(json!({
            "t1": { "labels": { "herdr.space": "sp1" } },
            "t2": { "labels": {}, "parent": "t1" },
            "t3": { "labels": {}, "parent": "t3" },
        }))
        .unwrap();
        assert_eq!(herdr_space_of(Some("t2"), &terms).as_deref(), Some("sp1"));
        assert_eq!(herdr_space_of(Some("t3"), &terms), None);
        assert_eq!(herdr_space_of(None, &terms), None);
    }

    #[test]
    fn a_space_does_not_match_a_longer_name() {
        let terms: Map<String, Value> = serde_json::from_value(json!({
            "t1": { "labels": { "herdr.space": "other" }, "cwd": "/t/foo-2/web" },
        }))
        .unwrap();
        assert_eq!(herdr_space_id(&json!({}), Path::new("/t/foo"), &terms), None);
        assert_eq!(herdr_space_id(&json!({}), Path::new("/t/foo-2"), &terms).as_deref(), Some("other"));
        assert_eq!(place_in("/t/foo-2/web", Path::new("/t/foo")), "/t/foo-2/web");
        assert_eq!(place_in("/t/foo/web/src", Path::new("/t/foo")), "web/src");
        assert_eq!(place_in("/t/foo", Path::new("/t/foo")), ".");
    }

    #[test]
    fn no_env_var_turns_the_terminal_check_off() {
        let _env = crate::util::env_lock();
        let before = std::env::var_os("HC_TERMINAL_ID");
        std::env::set_var("HC_TERMINAL_ID", "t9");
        std::env::set_var("HERDR_HELM_ALLOW_TERMINAL", "1");
        let refused = refuse_in_terminal("delete").is_err();
        std::env::remove_var("HC_TERMINAL_ID");
        let allowed = refuse_in_terminal("delete").is_ok();
        std::env::remove_var("HERDR_HELM_ALLOW_TERMINAL");
        if let Some(v) = before {
            std::env::set_var("HC_TERMINAL_ID", v);
        }
        assert!(refused && allowed);
    }
}
