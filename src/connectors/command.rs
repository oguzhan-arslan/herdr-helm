//! The command form of every kind: an external program that speaks the JSON contract in
//! connectors/mod.rs. `Command` holds the program and calls its ops. Each kind's trait is a thin
//! map from its methods to ops of the same name. docs/connectors*.md have the JSON of each op.
//!
//! The reply to "describe" is asked once per process, for each kind and program, and kept: name,
//! label and capabilities all come from it.

use super::agent::{Running, Session};
use super::code_host::{BranchPr, Checkout, CodeHost, PrTask};
use super::system::App;
use anyhow::{anyhow, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub struct Command {
    kind: &'static str,
    program: String,
}

impl Command {
    pub fn new(kind: &'static str, program: String) -> Self {
        Command { kind, program }
    }

    fn call(&self, op: &str, args: Value) -> Result<Value> {
        super::call_command(&self.program, self.kind, op, args)
    }

    /// Calls an op and reads its result as `T`.
    fn ask<T: DeserializeOwned>(&self, op: &str, args: Value) -> Result<T> {
        let reply = self.call(op, args)?;
        serde_json::from_value(reply).map_err(|e| anyhow!("{} connector {op}: unexpected reply: {e}", self.kind))
    }

    /// The text result of an op, or None when it fails or is not text.
    fn text(&self, op: &str, args: Value) -> Option<String> {
        self.call(op, args).ok().and_then(|v| v.as_str().map(String::from))
    }

    /// The list result of an op, or an empty list when it fails or is not a list.
    fn list(&self, op: &str, args: Value) -> Vec<Value> {
        self.call(op, args).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
    }

    /// The reply to "describe", asked once per process. A failed describe counts as null: the
    /// connector then has no capabilities.
    fn described(&self) -> Value {
        static CACHE: OnceLock<Mutex<HashMap<(String, String), Value>>> = OnceLock::new();
        let key = (self.kind.to_string(), self.program.clone());
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(v) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return v.clone();
        }
        let reply = self.call("describe", json!({})).unwrap_or(Value::Null);
        cache.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert(reply).clone()
    }

    fn name(&self) -> String {
        self.described()["name"].as_str().map(String::from).unwrap_or_else(|| self.program.clone())
    }

    fn capabilities(&self) -> Vec<String> {
        serde_json::from_value(self.described()["capabilities"].clone()).unwrap_or_default()
    }
}

// ------------------------------------------------------------------ code_host

impl CodeHost for Command {
    fn name(&self) -> String {
        Command::name(self)
    }
    fn capabilities(&self) -> Vec<String> {
        Command::capabilities(self)
    }
    fn host_of(&self, remote_url: &str) -> Option<String> {
        self.text("host_of", json!({ "remote_url": remote_url }))
    }
    fn org_repo(&self, remote_url: &str) -> Option<String> {
        self.text("org_repo", json!({ "remote_url": remote_url }))
    }
    fn branch_pr(&self, checkout: &Checkout) -> BranchPr {
        match self.call("branch_pr", json!(checkout)) {
            Err(e) => BranchPr::Error(e.to_string()),
            Ok(r) => branch_pr_of(&r),
        }
    }
    fn org_prs(&self) -> Result<Value> {
        self.call("org_prs", json!({}))
    }
    fn pr_info(&self, repo: &str, number: u64) -> Result<Value> {
        self.call("pr_info", json!({ "repo": repo, "number": number }))
    }
    fn pr_state(&self, repo: &str, number: u64) -> Option<String> {
        self.text("pr_state", json!({ "repo": repo, "number": number }))
    }
    fn branch_prs(&self, repo: &str, branch: &str) -> Vec<Value> {
        self.list("branch_prs", json!({ "repo": repo, "branch": branch }))
    }
    fn pr_ref(&self, url: &str) -> Option<(String, u64)> {
        let r = self.call("pr_ref", json!({ "url": url })).ok()?;
        Some((r["repo"].as_str()?.to_string(), r["number"].as_u64()?))
    }
    fn pr_at(&self, url: &str) -> Option<Value> {
        self.call("pr_at", json!({ "url": url })).ok().filter(Value::is_object)
    }
    fn enqueue(&self, repo: &str, number: u64) -> Result<String> {
        let r = self.call("enqueue", json!({ "repo": repo, "number": number }))?;
        Ok(r["message"].as_str().unwrap_or("Set to merge when ready.").to_string())
    }
    fn agent_prompt(&self, task: &PrTask) -> Result<String> {
        let r = self.call("agent_prompt", json!(task))?;
        r.as_str().map(String::from).ok_or_else(|| anyhow!("code_host connector agent_prompt: not a string"))
    }
}

/// {"pr": {...} | null, "not_hosted": bool, "error": "..." | null} -> BranchPr.
fn branch_pr_of(r: &Value) -> BranchPr {
    if let Some(e) = r["error"].as_str() {
        return BranchPr::Error(e.to_string());
    }
    if r["not_hosted"].as_bool() == Some(true) {
        return BranchPr::NotHosted;
    }
    match &r["pr"] {
        Value::Object(_) => BranchPr::Found(r["pr"].clone()),
        _ => BranchPr::None,
    }
}

// ------------------------------------------------------------------ agent

impl super::agent::Agent for Command {
    fn name(&self) -> String {
        Command::name(self)
    }
    fn icon(&self) -> Option<Value> {
        // "icon" of describe: {"path", "color"}, or a path alone.
        match &self.described()["icon"] {
            Value::String(p) => Some(json!({ "path": p, "color": null })),
            v @ Value::Object(_) => Some(v.clone()),
            _ => None,
        }
    }
    fn label(&self) -> String {
        self.described()["label"].as_str().map(String::from).unwrap_or_else(|| Command::name(self))
    }
    fn capabilities(&self) -> Vec<String> {
        Command::capabilities(self)
    }
    fn start_argv(&self) -> Result<Vec<String>> {
        self.start_argv_in(&[])
    }
    fn start_argv_in(&self, dirs: &[String]) -> Result<Vec<String>> {
        self.ask("start_argv", json!({ "dirs": dirs }))
    }
    fn programs(&self) -> Vec<String> {
        self.ask("programs", json!({})).unwrap_or_default()
    }
    fn running_session(&self, pid: i64) -> Result<Option<Running>> {
        self.ask("running_session", json!({ "pid": pid }))
    }
    fn resume_argv(&self, argv: &[String], session: &str) -> Result<Vec<String>> {
        self.ask("resume_argv", json!({ "argv": argv, "session": session }))
    }
    fn sessions_in(&self, roots: &[PathBuf]) -> Result<Vec<Session>> {
        let roots: Vec<String> = roots.iter().map(|r| r.to_string_lossy().into_owned()).collect();
        Ok(self.ask::<Option<Vec<Session>>>("sessions_in", json!({ "roots": roots }))?.unwrap_or_default())
    }
    fn sessions_by_id(&self, ids: &[String]) -> Result<Vec<Session>> {
        Ok(self.ask::<Option<Vec<Session>>>("sessions_by_id", json!({ "ids": ids }))?.unwrap_or_default())
    }
    fn smart_rename(&self, prompt: &str, schema: &Value) -> Result<Value> {
        self.call("smart_rename", json!({ "prompt": prompt, "schema": schema }))
    }
    fn usage(&self) -> Result<Value> {
        self.call("usage", json!({}))
    }
}

// ------------------------------------------------------------------ editor

impl super::editor::Editor for Command {
    fn name(&self) -> String {
        Command::name(self)
    }
    fn capabilities(&self) -> Vec<String> {
        Command::capabilities(self)
    }
    fn open(&self, path: &str) -> Result<()> {
        self.call("open", json!({ "path": path })).map(|_| ())
    }
    fn diff(&self, old: &str, new: &str, reuse: bool) -> Result<()> {
        self.call("diff", json!({ "old": old, "new": new, "reuse": reuse })).map(|_| ())
    }
    fn window(&self, folder: &str) -> Result<()> {
        self.call("window", json!({ "folder": folder })).map(|_| ())
    }
}

// ------------------------------------------------------------------ system

impl super::system::System for Command {
    fn name(&self) -> String {
        Command::name(self)
    }
    fn capabilities(&self) -> Vec<String> {
        Command::capabilities(self)
    }
    fn usage(&self) -> Result<Value> {
        self.call("usage", json!({}))
    }
    /// The program returns {"cores", "total", "apps": [{"name", "cpu", "kb", "count"}]}.
    fn apps(&self) -> Result<(Vec<App>, u64, u64)> {
        let r = self.call("apps", json!({}))?;
        let apps = r["apps"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|a| App {
                name: a["name"].as_str().unwrap_or("?").to_string(),
                cpu: a["cpu"].as_f64().unwrap_or(0.0),
                kb: a["kb"].as_f64().unwrap_or(0.0),
                count: a["count"].as_u64().unwrap_or(1),
            })
            .collect();
        Ok((apps, r["cores"].as_u64().unwrap_or(1), r["total"].as_u64().unwrap_or(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::agent::Agent;
    use crate::connectors::editor::Editor;
    use crate::connectors::system::System;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// A code host that saves each request and prints `reply`.
    fn fake(dir: &std::path::Path, reply: &str) -> (Command, std::path::PathBuf) {
        let log = dir.join("request.json");
        let program = script(dir, "host.sh", &format!("#!/bin/sh\ncat > '{}'\nprintf '%s' '{}'\n", log.display(), reply));
        (Command::new("code_host", program), log)
    }

    fn request(log: &std::path::Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(log).unwrap()).unwrap()
    }

    #[test]
    fn code_host_sends_kind_op_and_args() {
        let dir = tempfile::tempdir().unwrap();
        let (c, log) = fake(dir.path(), r#"{"repo":"web","number":7}"#);
        assert_eq!(c.pr_ref("https://example.com/acme/web/pull/7"), Some(("web".into(), 7)));
        let r = request(&log);
        assert_eq!(r["kind"], "code_host");
        assert_eq!(r["op"], "pr_ref");
        assert_eq!(r["args"], json!({ "url": "https://example.com/acme/web/pull/7" }));
        assert!(r.get("settings").is_some());
    }

    #[test]
    fn code_host_branch_pr_sends_the_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let (c, log) = fake(dir.path(), r#"{"pr":{"number":3},"not_hosted":false,"error":null}"#);
        let k = Checkout { path: "/x".into(), branch: "feat".into(), upstream: Some("origin/feat".into()), head: None };
        assert!(matches!(c.branch_pr(&k), BranchPr::Found(p) if p["number"] == 3));
        let r = request(&log);
        assert_eq!(r["op"], "branch_pr");
        assert_eq!(r["args"], json!({ "path": "/x", "branch": "feat", "upstream": "origin/feat", "head": null }));
    }

    #[test]
    fn code_host_enqueue_and_prompt_ops() {
        let dir = tempfile::tempdir().unwrap();
        let (c, log) = fake(dir.path(), r#"{"message":"queued"}"#);
        assert_eq!(c.enqueue("web", 9).unwrap(), "queued");
        assert_eq!(request(&log)["args"], json!({ "repo": "web", "number": 9 }));
        let (c, log) = fake(dir.path(), r#""Review it""#);
        let task = PrTask {
            action: "review".into(),
            repo: "web".into(),
            number: 9,
            title: "t".into(),
            url: "u".into(),
            head: "h".into(),
            base: "main".into(),
            path: "/p".into(),
            notes: "/n".into(),
            reasons: "see the PR".into(),
        };
        assert_eq!(c.agent_prompt(&task).unwrap(), "Review it");
        let r = request(&log);
        assert_eq!(r["op"], "agent_prompt");
        assert_eq!(r["args"]["action"], "review");
        assert_eq!(r["args"]["number"], 9);
    }

    #[test]
    fn a_failing_connector_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(dir.path(), "bad.sh", "#!/bin/sh\necho 'no such PR' >&2\nexit 3\n");
        let c = Command::new("code_host", program);
        let e = c.pr_info("web", 1).unwrap_err().to_string();
        assert!(e.contains("pr_info") && e.contains("no such PR"), "{e}");
        assert!(matches!(c.branch_pr(&Checkout { path: "/x".into(), branch: "b".into(), upstream: None, head: None }), BranchPr::Error(_)));
        // A failed describe gives no capabilities, and the program as the name.
        assert!(CodeHost::capabilities(&c).is_empty());
    }

    #[test]
    fn branch_pr_reply_mapping() {
        assert!(matches!(branch_pr_of(&json!({ "pr": null })), BranchPr::None));
        assert!(matches!(branch_pr_of(&json!({ "pr": null, "not_hosted": true })), BranchPr::NotHosted));
        assert!(matches!(branch_pr_of(&json!({ "error": "boom" })), BranchPr::Error(e) if e == "boom"));
    }

    /// An agent connector that keeps each request in a file and answers by op.
    fn agent(dir: &std::path::Path) -> Command {
        let log = dir.join("requests");
        let body = format!(
            r#"#!/bin/sh
req=$(cat)
printf '%s\n' "$req" >> '{log}'
case "$req" in
  *'"op":"describe"'*) echo '{{"name":"toy","label":"Toy","capabilities":["start","resume"]}}' ;;
  *'"op":"start_argv"'*) echo '["toy","--dir","/x"]' ;;
  *'"op":"running_session"'*) echo 'null' ;;
  *'"op":"sessions_by_id"'*) echo '[{{"session":"s1","messages":2}}]' ;;
  *) echo "unknown op" >&2; exit 2 ;;
esac
"#,
            log = log.display()
        );
        Command::new("agent", script(dir, "agent.sh", &body))
    }

    #[test]
    fn agent_speaks_the_json_contract() {
        let _env = crate::util::env_lock();
        let dir = tempfile::tempdir().unwrap();
        // The settings come from this config, not the person's.
        let config = dir.path().join("projects.json");
        std::fs::write(&config, r#"{"agent_settings": {"k": 1}}"#).unwrap();
        std::env::set_var("HERDR_PROJECTS", &config);
        let a = agent(dir.path());

        assert_eq!(Agent::name(&a), "toy");
        assert_eq!(a.label(), "Toy");
        assert_eq!(Agent::capabilities(&a), vec!["start".to_string(), "resume".to_string()]);
        assert_eq!(a.start_argv_in(&["/x".into()]).unwrap(), vec!["toy", "--dir", "/x"]);
        assert_eq!(a.running_session(42).unwrap(), None);
        let sessions = a.sessions_by_id(&["s1".into()]).unwrap();
        assert_eq!(sessions[0].session, "s1");
        assert_eq!(sessions[0].messages, 2);
        let err = Agent::usage(&a).unwrap_err().to_string();
        assert!(err.contains("agent connector usage: unknown op"), "{err}");
        // A second connector object of the same program does not ask describe again.
        assert_eq!(Agent::name(&agent(dir.path())), "toy");

        let requests: Vec<Value> =
            std::fs::read_to_string(dir.path().join("requests")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        // describe is asked once per process, for name, label and capabilities.
        assert_eq!(requests.iter().filter(|r| r["op"] == "describe").count(), 1);
        let start = requests.iter().find(|r| r["op"] == "start_argv").unwrap();
        assert_eq!(start, &json!({ "kind": "agent", "op": "start_argv", "args": { "dirs": ["/x"] }, "settings": { "k": 1 } }));
        let running = requests.iter().find(|r| r["op"] == "running_session").unwrap();
        assert_eq!(running["args"], json!({ "pid": 42 }));
    }

    #[test]
    fn editor_and_system_ops() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("requests");
        let body = format!(
            r#"#!/bin/sh
req=$(cat)
printf '%s\n' "$req" >> '{log}'
case "$req" in
  *'"op":"describe"'*) echo '{{"name":"ed","capabilities":["open","apps"]}}' ;;
  *'"op":"apps"'*) echo '{{"cores":4,"total":8,"apps":[{{"name":"a","cpu":1.5,"kb":10,"count":2}}]}}' ;;
  *) echo null ;;
esac
"#,
            log = log.display()
        );
        let program = script(dir.path(), "both.sh", &body);
        let e = Command::new("editor", program.clone());
        assert_eq!(Editor::name(&e), "ed");
        e.diff("/a", "/b", true).unwrap();
        let s = Command::new("system", program);
        let (apps, cores, total) = s.apps().unwrap();
        assert_eq!((apps[0].name.as_str(), apps[0].count, cores, total), ("a", 2, 4, 8));
        assert_eq!(System::capabilities(&s), vec!["open".to_string(), "apps".to_string()]);
        let requests: Vec<Value> = std::fs::read_to_string(&log).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let diff = requests.iter().find(|r| r["op"] == "diff").unwrap();
        assert_eq!((diff["kind"].as_str(), &diff["args"]), (Some("editor"), &json!({ "old": "/a", "new": "/b", "reuse": true })));
        // The cache is by kind and program: the system kind asks describe on its own.
        assert_eq!(requests.iter().filter(|r| r["op"] == "describe").count(), 2);
    }
}
