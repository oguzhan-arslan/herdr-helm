//! Connectors: Helm's only way to reach an external service. The core and the mods never call a
//! service directly; they call a connector of one of four kinds:
//!
//!   code_host  PRs of a branch, the org's open PRs, review, fix, merge queue   (built in: github)
//!   agent      start, resume, sessions, Smart rename, usage limits            (built in: claude-code)
//!   editor     open a file, a diff, a folder                                   (built in: vscode)
//!   system     CPU, memory, the apps that use the most                         (built in: macos)
//!
//! projects.json picks one per kind in "connectors": a built-in by name, or an external program
//! of any language as {"command": "<program>"}. A connector's settings are in "<kind>_settings"
//! (a built-in may also read an older block, e.g. "github"). The command form speaks JSON:
//!
//!   stdin:  {"kind": "code_host", "op": "<method>", "args": {...}, "settings": {...}}
//!   stdout: the method's result as JSON. A non-zero exit is an error; stderr is its message.
//!   op "describe" returns {"name": "...", "capabilities": ["..."]}, asked once per process.
//!
//! Each kind's module has the trait, the built-ins, `current()` (the chosen connector) and
//! `describe()`. The command form of all kinds is in command.rs. docs/connectors.md describes
//! every method and its JSON.

pub mod agent;
pub mod code_host;
mod command;
pub mod editor;
pub mod system;

use crate::util::{exec_with, expand, read_json, CALL_TIMEOUT};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// The connector picked for a kind.
pub enum Choice {
    /// A built-in, by name ("github", "claude-code", ...); "none" turns the kind off.
    Builtin(String),
    /// An external program that speaks the JSON contract.
    Command(String),
}

/// projects.json "connectors.<kind>", else the default built-in.
pub fn choice(kind: &str, default: &str) -> Choice {
    let config = read_json(&crate::paths::config_file(), json!({}));
    match config.get("connectors").and_then(|c| c.get(kind)) {
        Some(Value::String(name)) => Choice::Builtin(name.clone()),
        Some(Value::Object(o)) => match o.get("command").and_then(Value::as_str) {
            Some(cmd) => Choice::Command(cmd.to_string()),
            None => Choice::Builtin(default.to_string()),
        },
        _ => Choice::Builtin(default.to_string()),
    }
}

/// projects.json "<kind>_settings" (an object), or {}.
pub fn settings(kind: &str) -> Value {
    let config = read_json(&crate::paths::config_file(), json!({}));
    config.get(format!("{kind}_settings")).cloned().unwrap_or_else(|| json!({}))
}

/// Calls an external connector: one JSON request on stdin, one JSON result on stdout. The call
/// stops after `CALL_TIMEOUT`. The error message is all of stderr, trimmed.
pub fn call_command(program: &str, kind: &str, op: &str, args: Value) -> Result<Value> {
    let request = json!({ "kind": kind, "op": op, "args": args, "settings": settings(kind) });
    let parts = command_words(program)?;
    let (exe, rest) = parts.split_first().ok_or_else(|| anyhow!("empty connector command"))?;
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    let out =
        exec_with(exe, &rest, None, Some(&request.to_string()), Some(CALL_TIMEOUT)).map_err(|e| anyhow!("{kind} connector {op}: {e}"))?;
    if !out.ok() {
        let why = out.stderr.trim();
        return Err(anyhow!("{kind} connector {op}: {}", if why.is_empty() { format!("exit {}", out.code) } else { why.to_string() }));
    }
    let text = out.stdout.trim();
    if text.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(text).map_err(|e| anyhow!("{kind} connector {op}: not JSON: {e}"))
}

/// The words of a connector command line. A word that is "~" or starts with "~/" gets the home
/// folder, as in a shell, so "~/bin/x" and "python3 ~/helm/x.py" work.
fn command_words(program: &str) -> Result<Vec<String>> {
    let parts = shlex::split(program).ok_or_else(|| anyhow!("connector command is not a valid command line: {program}"))?;
    Ok(parts.into_iter().map(|w| if w == "~" || w.starts_with("~/") { expand(&w).to_string_lossy().into_owned() } else { w }).collect())
}

/// The chosen connector of every kind and what it can do, for the mods: the UI hides what a
/// connector cannot do.
pub fn describe_all() -> Value {
    json!({
        "code_host": code_host::describe(),
        "agent": agent::describe(),
        "editor": editor::describe(),
        "system": system::describe(),
    })
}

/// {"name": ..., "capabilities": [...]} of a connector that is off or missing.
pub fn none() -> Value {
    json!({ "name": "none", "capabilities": [] })
}

/// Whether a list of capabilities has `capability`.
pub fn has(capabilities: &[String], capability: &str) -> bool {
    capabilities.iter().any(|c| c == capability)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_words_expand_the_home_folder() {
        let _env = crate::util::env_lock();
        let home = crate::util::home().to_string_lossy().into_owned();
        assert_eq!(command_words("~/bin/x --flag").unwrap(), vec![format!("{home}/bin/x"), "--flag".into()]);
        assert_eq!(command_words("python3 ~/helm/gitlab.py").unwrap(), vec!["python3".to_string(), format!("{home}/helm/gitlab.py")]);
        assert_eq!(command_words("~").unwrap(), vec![home.clone()]);
        // Only a leading "~" or "~/": "~user" and a "~" inside a word stay.
        assert_eq!(command_words("~ann/x a~/b").unwrap(), vec!["~ann/x".to_string(), "a~/b".into()]);
        assert!(command_words("'open").is_err());
    }

    #[test]
    fn call_command_runs_a_program_under_home() {
        let _env = crate::util::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = bin.join("c.sh");
        std::fs::write(&script, "#!/bin/sh\ncat > /dev/null\necho '\"ok\"'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (old_home, old_cfg) = (std::env::var_os("HOME"), std::env::var_os("HERDR_PROJECTS"));
        std::env::set_var("HOME", dir.path());
        std::env::set_var("HERDR_PROJECTS", dir.path().join("projects.json"));
        let got = call_command("~/bin/c.sh", "editor", "open", json!({}));
        match old_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match old_cfg {
            Some(c) => std::env::set_var("HERDR_PROJECTS", c),
            None => std::env::remove_var("HERDR_PROJECTS"),
        }
        assert_eq!(got.unwrap(), json!("ok"));
    }
}
