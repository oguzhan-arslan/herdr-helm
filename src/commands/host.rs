//! open (the editor connector), system (the system connector), and connectors (the overview of
//! all kinds and their capabilities, which the mods read to hide what is not there).

use crate::cli::*;
use crate::connectors::{editor, system};
use crate::util::{now_ms, out};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn cmd_open(c: &Ctx, a: &OpenArgs) -> Result<()> {
    if let [repo, file] = a.change.as_slice() {
        let (old, new) = diff_pair(repo, file, &cache_dir(), 1)?;
        editor::current_with("diff")?.diff(&old, &new, false)?;
        return done(c, "diff");
    }
    if let [repo, files @ ..] = a.changes.as_slice() {
        // Check before the background run, so that the person sees the error.
        let editor = window_editor()?;
        if !a.now {
            return in_background(c);
        }
        let dir = cache_dir();
        editor.window(repo)?;
        // The new window needs a moment before it takes the diffs.
        std::thread::sleep(std::time::Duration::from_secs(2));
        for (i, file) in files.iter().enumerate() {
            let (old, new) = diff_pair(repo, file, &dir, i + 1)?;
            editor.diff(&old, &new, true)?;
        }
        return done(c, "diffs");
    }
    let path = a.path.as_deref().ok_or_else(|| anyhow!("give a file, --change REPO STATUS:PATH, or --changes REPO STATUS:PATH..."))?;
    editor::current_with("open")?.open(path)?;
    done(c, "open")
}

/// The editor connector for Open all: it needs "window" and "diff".
fn window_editor() -> Result<Box<dyn editor::Editor>> {
    let editor = editor::current_with("window")?;
    if !crate::connectors::has(&editor.capabilities(), "diff") {
        anyhow::bail!("the editor connector '{}' cannot do 'diff'", editor.name());
    }
    Ok(editor)
}

fn done(c: &Ctx, what: &str) -> Result<()> {
    out(&json!({ "ok": true, "did": what }), "", c.json);
    Ok(())
}

/// Runs this same command again with --now, detached, and returns at once.
fn in_background(c: &Ctx) -> Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1).chain(std::iter::once("--now".to_string())).collect();
    std::process::Command::new(exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    done(c, "started")
}

fn cache_dir() -> PathBuf {
    crate::paths::state_dir().join("diff").join(format!("{:x}", now_ms()))
}

/// The two sides of a changed file's diff against HEAD. Git writes the HEAD side to a cache file;
/// a new file compares against an empty one, and a deleted file's new side is an empty one.
fn diff_pair(repo: &str, file: &str, dir: &Path, i: usize) -> Result<(String, String)> {
    let (status, path) = file.split_once(':').ok_or_else(|| anyhow!("not STATUS:PATH: {file}"))?;
    std::fs::create_dir_all(dir)?;
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let old = dir.join(format!("{i}-HEAD-{name}"));
    let head = crate::util::git_maybe(Path::new(repo), &["show", &format!("HEAD:{path}")], true).unwrap_or_default();
    std::fs::write(&old, head)?;
    let new = if status == "D" {
        let empty = dir.join(format!("{i}-deleted-{name}"));
        std::fs::write(&empty, "")?;
        empty
    } else {
        Path::new(repo).join(path)
    };
    Ok((old.to_string_lossy().into_owned(), new.to_string_lossy().into_owned()))
}

pub fn cmd_system(c: &Ctx, a: &SystemArgs) -> Result<()> {
    let (data, text) = if a.apps {
        let (apps, cores, total) = system::current_with("apps")?.apps()?;
        let data = system::ranked(apps, cores, total, 14);
        let text = apps_text(&data);
        (data, text)
    } else {
        let data = system::current_with("usage")?.usage()?;
        let text = usage_text(&data);
        (data, text)
    };
    out(&data, &text, c.json);
    Ok(())
}

/// A size in bytes for a person: "16 GB", "512 MB".
fn size_text(bytes: f64) -> String {
    let gb = bytes / f64::from(1u32 << 30);
    if gb >= 1.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{:.0} MB", bytes / f64::from(1u32 << 20))
    }
}

fn usage_text(d: &Value) -> String {
    let pct = |k: &str| d[k].as_f64().map(|v| format!("{v:.0}%")).unwrap_or_else(|| "-".into());
    let total = d["total"].as_f64().map(|t| format!(" of {}", size_text(t))).unwrap_or_default();
    format!("CPU {}  Memory {}{total}", pct("cpu"), pct("mem"))
}

fn apps_text(d: &Value) -> String {
    let mut lines = vec![format!("{} cores, {} memory", d["cores"], size_text(d["total"].as_f64().unwrap_or(0.0)))];
    for a in d["apps"].as_array().into_iter().flatten() {
        let count = a["count"].as_u64().unwrap_or(1);
        let procs = if count > 1 { format!("  ({count} processes)") } else { String::new() };
        let name = a["name"].as_str().unwrap_or("?");
        let mem = size_text(a["kb"].as_f64().unwrap_or(0.0) * 1024.0);
        lines.push(format!("{name:<28} CPU {:>5.1}%  {mem:>7}{procs}", a["cpu"].as_f64().unwrap_or(0.0)));
    }
    lines.join("\n")
}

pub fn cmd_connectors(c: &Ctx) -> Result<()> {
    let d = crate::connectors::describe_all();
    let text = ["code_host", "agent", "editor", "system"]
        .iter()
        .map(|k| {
            let caps = d[k]["capabilities"]
                .as_array()
                .map(|c| c.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            format!("{k}: {} ({caps})", d[k]["name"].as_str().unwrap_or("none"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    out(&d, &text, c.json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_pair_of_a_deleted_file_is_empty_on_the_new_side() {
        let dir = tempfile::tempdir().expect("tmp");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        let (old, new) = diff_pair(repo.to_str().expect("utf8"), "D:a/b.txt", &dir.path().join("c"), 3).expect("pair");
        assert!(old.ends_with("3-HEAD-b.txt"));
        assert!(new.ends_with("3-deleted-b.txt"));
        assert_eq!(std::fs::read_to_string(new).expect("read"), "");
    }

    #[test]
    fn system_text_without_json() {
        let usage = json!({ "cpu": 12.4, "mem": 61.0, "total": 34359738368u64 });
        assert_eq!(usage_text(&usage), "CPU 12%  Memory 61% of 32 GB");
        let apps = json!({ "cores": 8, "total": 17179869184u64, "apps": [{ "name": "Code", "cpu": 3.25, "kb": 524288, "count": 4 }] });
        let text = apps_text(&apps);
        assert!(text.starts_with("8 cores, 16 GB memory\nCode"), "{text}");
        assert!(text.contains("CPU   3.2%") && text.contains("512 MB") && text.ends_with("(4 processes)"), "{text}");
    }

    #[test]
    fn diff_pair_of_a_changed_file_is_the_file_itself() {
        let dir = tempfile::tempdir().expect("tmp");
        let (_, new) = diff_pair("/r", "M:x/y.rs", &dir.path().join("c"), 1).expect("pair");
        assert_eq!(new, "/r/x/y.rs");
    }
}
