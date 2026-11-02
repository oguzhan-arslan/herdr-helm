//! The commands of the agent connector: agents, spawn, send, wait, sessions, restart, rename,
//! info, usage. What is special to one agent (Claude Code, ...) is behind the Agent trait in
//! connectors/agent. Here is the generic part: the agents of a space through hc, the record of
//! running sessions, the titles of a space and their history, and the info of a space.

use crate::cli::*;
use crate::connectors::agent::{self, Agent, Session};
use crate::core::herdr::{hc, hc_maybe, herdr, herdr_space_id, space_agents, terminals};
use crate::fail;
use crate::paths::state_dir;
use crate::util::{exec, git_maybe, locked, now_ms, now_secs, out, read_json, run, tool, write_json, READ};
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::connectors::code_host::link_space_prs;
use crate::core::{checkout_status, config, load_space, prs_of, record_path, tasks_root, update_space, write_agent_files};

/// The programs that herdr-core puts in place of an agent that a shutdown ended.
const SHELLS: [&str; 7] = ["zsh", "bash", "sh", "fish", "dash", "ksh", "tcsh"];

/// How many old titles a space keeps.
const TITLES_KEPT: usize = 20;

// ---------------------------------------------------------------- agents, send, spawn, wait

pub fn cmd_agents(c: &Ctx) -> Result<()> {
    let (rec, space_dir) = load_space(&config()?, c.space.as_deref())?;
    let (sid, agents) = space_agents(&rec, &space_dir)?;
    if sid.is_none() {
        fail!("This space has no Herdr space yet, so it has no agents.");
    }
    let text = agents.iter().map(agent_line).collect::<Vec<_>>().join("\n");
    out(&Value::Array(agents), &text, c.json);
    Ok(())
}

/// A text, or "-" when it is missing or empty.
fn or_dash(v: &Value) -> &str {
    v.as_str().filter(|s| !s.is_empty()).unwrap_or("-")
}

fn agent_line(x: &Value) -> String {
    let you = if x["you"].as_bool().unwrap_or(false) { "  (you)" } else { "" };
    format!(
        "{:<5} {:<24} {:<9} {:<8} {}{you}",
        or_dash(&x["address"]),
        or_dash(&x["name"]),
        or_dash(&x["role"]),
        or_dash(&x["status"]),
        x["where"].as_str().unwrap_or("")
    )
}

fn find_agent(rec: &Value, space_dir: &Path, target: &str) -> Result<Value> {
    let (_, agents) = space_agents(rec, space_dir)?;
    let space = rec["name"].as_str().unwrap_or("");
    let names = [target.to_string(), format!("{space}-{target}")];
    match agents.into_iter().find(|x| x["address"] == target || x["name"].as_str().is_some_and(|n| names.iter().any(|m| m == n))) {
        Some(found) => Ok(found),
        None => Err(anyhow!("No agent '{target}' in space {space}. Run 'herdr-helm agents' to see them.")),
    }
}

pub fn cmd_send(c: &Ctx, a: &SendArgs) -> Result<()> {
    let (rec, space_dir) = load_space(&config()?, c.space.as_deref())?;
    let target = find_agent(&rec, &space_dir, &a.agent)?;
    let addr = target["address"].as_str().unwrap_or("");
    hc(&["send", addr, &a.text])?;
    out(&json!({ "sent": addr }), &format!("Sent to {addr}."), c.json);
    Ok(())
}

fn valid_agent_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit());
    first_ok && name.len() <= 8 && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

/// 32 random hex digits from /dev/urandom, for the id of a new tab.
fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes));
    if read.is_err() {
        // No /dev/urandom: the time and the process id are unique enough for a tab id.
        return format!("{:016x}{:016x}", now_ms(), std::process::id());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn cmd_spawn(c: &Ctx, a: &SpawnArgs) -> Result<()> {
    let (rec, space_dir) = load_space(&config()?, c.space.as_deref())?;
    if !valid_agent_name(&a.name) {
        fail!("An agent name is 1-8 of a-z 0-9 -, and starts with a letter or digit.");
    }
    let space = rec["name"].as_str().unwrap_or("");
    let full: String = format!("{space}-{}", a.name).chars().take(32).collect();
    let work = match &a.checkout {
        None => space_dir.clone(),
        Some(folder) => {
            let found = rec["checkouts"].as_array().into_iter().flatten().find(|k| k["folder"] == folder.as_str());
            match found.and_then(|k| k["path"].as_str()) {
                Some(path) => PathBuf::from(path),
                None => fail!("No checkout '{folder}'. Add it first with: herdr-helm add <repo> --as <label>"),
            }
        }
    };
    let prompt = spawn_prompt(&a.name, space, &work, &space_dir, a.checkout.is_some(), &a.task);
    // A handoff agent always gets a tab in its own space, so the person can watch and steer it.
    // A tab id Herdr does not know becomes a new tab in that space. terminal.keep would put the
    // tab in whatever space is on screen.
    let Some(sid) = herdr_space_id(&rec, &space_dir, &terminals()?) else {
        fail!("The space {space} is not open in Herdr, so a handoff agent gets no tab. Open the space in Herdr first.");
    };
    let tab = format!("tb{}", random_hex());
    let labels: Vec<String> =
        ["--label", "herdr.role=handoff", "--label", &format!("herdr.space={sid}"), "--label", &format!("herdr.tab={tab}")]
            .map(String::from)
            .to_vec();
    let argv = agent::space_argv(&rec)?;
    let dir = space_dir.to_string_lossy();
    let mut args: Vec<&str> = vec!["run", "--bg", "--json", "--name", &full, "--cwd", &dir, "--outlive-parent"];
    args.extend(labels.iter().map(String::as_str));
    args.push("--");
    args.extend(argv.iter().map(String::as_str));
    args.push(&prompt);
    let result: Value = serde_json::from_str(&hc(&args)?).map_err(|e| anyhow!("hc run gave no JSON: {e}"))?;
    name_tab(&tab, &a.name);
    let addr = result["address"].as_str().unwrap_or("-").to_string();
    out(&result, &format!("Started {full} as {addr} in the tab '{}'. Wait for its report with: herdr-helm wait", a.name), c.json);
    Ok(())
}

fn spawn_prompt(name: &str, space: &str, work: &Path, space_dir: &Path, own_checkout: bool, task: &str) -> String {
    let report = match std::env::var("HC_TERMINAL_ID") {
        Ok(me) => format!("herdr-helm send {me} \"<result or blocker>\""),
        Err(_) => "herdr-helm agents (find who started you)".into(),
    };
    let more = if own_checkout { ". You own this checkout and can change its files freely" } else { " and the checkouts the task names" };
    format!(
        "You are the handoff agent '{name}' in the Herdr space '{space}'. Read the task-workspace skill first. \
         Work only in {}{more}. Do not start other handoff agents unless the task says so. \
         You can start your own subagents with the Agent tool, and for a big task you should: give each one \
         a part of the work in your folder. \
         When you are done or stuck, report with: {report}. \
         Put long results in {}/.notes/{name}.md and send only the path.\n\nTask: {task}",
        work.display(),
        space_dir.display()
    )
}

/// Herdr makes the tab a moment after the terminal starts, so the rename may need a few tries.
fn name_tab(id: &str, name: &str) {
    for _ in 0..10 {
        if herdr("tab.rename", &json!({ "id": id, "name": name })).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

pub fn cmd_wait(_c: &Ctx) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = tool("hc");
    let err = std::process::Command::new(&exe).args(["wait", "--children", "any"]).exec();
    Err(anyhow!("{exe}: {err}"))
}

// ---------------------------------------------------------------- sessions and restart
// The agent sessions that a shutdown ended, so Herdr can resume them: `sessions save` keeps
// the session of each terminal in state_dir/sessions.json, `sessions resume` starts them again.

fn sessions_file() -> PathBuf {
    state_dir().join("sessions.json")
}

fn load_sessions() -> Map<String, Value> {
    match read_json(&sessions_file(), json!({})) {
        Value::Object(o) => o,
        _ => Map::new(),
    }
}

fn save_sessions(rec: &Map<String, Value>) -> Result<()> {
    write_json(&sessions_file(), &Value::Object(rec.clone()))
}

/// When herdr-core last started, in ms. A session recorded before it was ended by the
/// restart (a shutdown, a crash), not by the person.
fn core_start() -> Result<i64> {
    let status: Value = serde_json::from_str(&hc(&["server", "status", "--json"])?)?;
    let up = status["uptime_ms"].as_i64().ok_or_else(|| anyhow!("hc server status has no uptime_ms"))?;
    Ok(now_ms() - up)
}

fn basename(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().trim_start_matches('-').to_string()).unwrap_or_default()
}

/// The program in the front of a terminal, e.g. "claude" or "zsh".
fn program(t: &Value) -> String {
    t["foreground"]["argv"][0].as_str().map(basename).unwrap_or_default()
}

fn runs(t: &Value, programs: &[String]) -> bool {
    let p = program(t);
    !p.is_empty() && programs.contains(&p)
}

/// A word for the shell: as it is when it has only safe characters, else in single quotes.
fn sh_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = |ch: char| ch.is_ascii_alphanumeric() || "_@%+=:,./-".contains(ch);
    if s.chars().all(safe) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

fn sh_join(argv: &[String]) -> String {
    argv.iter().map(|x| sh_quote(x)).collect::<Vec<_>>().join(" ")
}

pub fn cmd_sessions(c: &Ctx, a: &SessionsArgs) -> Result<()> {
    let need = if a.action == "save" { "sessions" } else { "resume" };
    let agent = agent::current_with(need)?;
    let terms = terminals()?;
    let start = core_start()?;
    // The read, the change and the write of sessions.json happen under its lock.
    if a.action == "save" {
        let rec = locked(&sessions_file(), || {
            let mut rec = load_sessions();
            save_running(agent.as_ref(), &mut rec, &terms, start)?;
            Ok(rec)
        })?;
        let text = format!("{} {} sessions recorded.", rec.len(), agent.label());
        out(&Value::Object(rec), &text, c.json);
        return Ok(());
    }
    let resumed = locked(&sessions_file(), || {
        let mut rec = load_sessions();
        let resumed = resume_ended(agent.as_ref(), &mut rec, &terms, start);
        save_sessions(&rec)?;
        Ok(resumed)
    })?;
    let text =
        resumed.iter().map(|r| format!("Resumed {} in {}", or_dash(&r["session"]), or_dash(&r["address"]))).collect::<Vec<_>>().join("\n");
    out(&json!({ "resumed": resumed }), if text.is_empty() { "Nothing to resume." } else { &text }, c.json);
    Ok(())
}

/// Records the session of each terminal where the agent runs, and forgets those the person ended.
fn save_running(agent: &dyn Agent, rec: &mut Map<String, Value>, terms: &Map<String, Value>, start: i64) -> Result<()> {
    let programs = agent.programs();
    let now = now_ms();
    for (addr, t) in terms {
        if !runs(t, &programs) {
            continue;
        }
        let Some(pid) = t["foreground"]["pid"].as_i64() else { continue };
        let Some(s) = agent.running_session(pid)? else { continue };
        let cwd = s.cwd.map(Value::from).unwrap_or_else(|| t["cwd"].clone());
        rec.insert(addr.clone(), json!({ "session": s.session, "cwd": cwd, "name": t["name"], "at": now }));
    }
    // Gone, or the agent ended while the core ran: the person ended it. A record from before
    // the core started waits for `resume`.
    rec.retain(|addr, e| match terms.get(addr) {
        None => false,
        Some(t) => runs(t, &programs) || e["at"].as_i64().unwrap_or(0) < start,
    });
    save_sessions(rec)
}

/// Resumes each session that the last shutdown ended, in the shell that took its place. A
/// record goes only when `hc send` worked, or when its terminal is gone or runs another
/// program. So a failed send keeps the session for the next try.
fn resume_ended(agent: &dyn Agent, rec: &mut Map<String, Value>, terms: &Map<String, Value>, start: i64) -> Vec<Value> {
    let ended: Vec<String> = rec.iter().filter(|(_, e)| e["at"].as_i64().unwrap_or(0) < start).map(|(a, _)| a.clone()).collect();
    let Ok(base) = agent.start_argv() else { return Vec::new() };
    let mut resumed = Vec::new();
    for addr in ended {
        let Some(e) = rec.get(&addr).cloned() else { continue };
        // Resume only into the plain shell that herdr-core put in place of the agent.
        if !terms.get(&addr).is_some_and(|t| SHELLS.contains(&program(t).as_str())) {
            rec.remove(&addr);
            continue;
        }
        let Ok(argv) = agent.resume_argv(&base, e["session"].as_str().unwrap_or("")) else { continue };
        let cmd = format!("cd {} && {}", sh_quote(e["cwd"].as_str().unwrap_or("")), sh_join(&argv));
        if hc_maybe(&["send", &addr, &cmd]).is_none() {
            continue;
        }
        rec.remove(&addr);
        let mut row = Map::new();
        row.insert("address".into(), json!(addr));
        if let Value::Object(fields) = e {
            row.extend(fields);
        }
        resumed.push(Value::Object(row));
    }
    resumed
}

/// Restarts the agent of a terminal in the same session, for example after an update.
/// In a shell: the agent stops (SIGTERM) and the shell gets the same command, resumed.
/// The agent as the terminal's own command: a new tab resumes it, and the old terminal ends.
pub fn cmd_restart(c: &Ctx, a: &RestartArgs) -> Result<()> {
    let agent = agent::current_with("restart")?;
    let programs = agent.programs();
    let label = agent.label();
    let term = a.terminal.as_str();
    let t = terminals()?.get(term).cloned().unwrap_or(Value::Null);
    if !runs(&t, &programs) {
        fail!("No {label} runs in {term}.");
    }
    let fg = &t["foreground"];
    let pid = fg["pid"].as_i64().ok_or_else(|| anyhow!("{label} in {term} has no process id."))?;
    let Some(s) = agent.running_session(pid)? else {
        fail!("{label} in {term} has no session file yet. Send it a message first.");
    };
    let cwd = s.cwd.clone().or_else(|| fg["cwd"].as_str().map(String::from)).or_else(|| t["cwd"].as_str().map(String::from));
    let now: Vec<String> = serde_json::from_value(fg["argv"].clone()).unwrap_or_default();
    let now = if now.is_empty() { agent.start_argv()? } else { now };
    let argv = agent.resume_argv(&now, &s.session)?;
    let own = !SHELLS.contains(&t["argv"][0].as_str().map(basename).unwrap_or_default().as_str());
    if own {
        let new = herdr("tab.new", &json!({ "space": t["labels"]["herdr.space"], "folder": cwd, "argv": argv }))?;
        let _ = hc_maybe(&["kill", term]);
        let data = json!({ "terminal": term, "session": s.session, "new": new["terminal"] });
        out(&data, &format!("Resumed {} in {}; {term} ended.", s.session, or_dash(&new["terminal"])), c.json);
        return Ok(());
    }
    stop(pid, term, &programs, &label)?;
    let mut cmd = sh_join(&argv);
    let shell_cwd = terminals().ok().and_then(|ts| ts.get(term).and_then(|t| t["cwd"].as_str().map(String::from)));
    if let Some(dir) = cwd.filter(|d| !d.is_empty() && Some(d) != shell_cwd.as_ref()) {
        cmd = format!("cd {} && {cmd}", sh_quote(&dir));
    }
    hc(&["send", term, &cmd])?;
    out(&json!({ "terminal": term, "session": s.session, "new": null }), &format!("Resumed {} in {term}.", s.session), c.json);
    Ok(())
}

/// Stops the agent process and waits up to 10 seconds until its terminal shows another program.
fn stop(pid: i64, term: &str, programs: &[String], label: &str) -> Result<()> {
    let killed = exec("kill", &["-TERM", &pid.to_string()], None, None)?;
    if !killed.ok() {
        fail!("Could not stop {label} in {term}: {}", killed.stderr.trim());
    }
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(250));
        let t = terminals().ok().and_then(|ts| ts.get(term).cloned()).unwrap_or(Value::Null);
        if !runs(&t, programs) {
            return Ok(());
        }
    }
    fail!("{label} in {term} did not stop. Nothing was restarted.")
}

// ---------------------------------------------------------------- titles
// A space has a title: the one given at the start, then the title of a PR of its branches,
// or of its agent sessions, or of Smart rename. `title_source` says which, `titles` keeps the
// old ones.

/// The source of a title, for a person.
fn title_source_text(source: &str) -> Option<&'static str> {
    match source {
        "user" => Some("you"),
        "pr" => Some("a PR"),
        "session" => Some("an agent session"),
        "smart" => Some("Smart rename"),
        _ => None,
    }
}

fn title_source(rec: &Value) -> String {
    rec.get("title_source").and_then(Value::as_str).unwrap_or("user").to_string()
}

/// The title of a space: its own, else its name.
fn title_of(rec: &Value) -> String {
    rec["title"].as_str().filter(|t| !t.is_empty()).or_else(|| rec["name"].as_str()).unwrap_or("").to_string()
}

/// Adds a title to the history. A record without a history starts it with its current title.
fn push_title(rec: &mut Value, title: &str, source: &str) {
    let mut titles = rec["titles"].as_array().cloned().unwrap_or_default();
    if titles.is_empty() {
        titles.push(json!({ "title": title_of(rec), "source": title_source(rec), "at": rec["created"] }));
    }
    titles.push(json!({ "title": title, "source": source, "at": now_secs() }));
    let extra = titles.len().saturating_sub(TITLES_KEPT);
    titles.drain(..extra);
    rec["titles"] = Value::Array(titles);
    rec["title"] = json!(title);
    rec["title_source"] = json!(source);
}

/// Gives the space a new title, in the record, in CLAUDE.md, and in Herdr.
fn set_title(rec: &mut Value, space_dir: &Path, title: &str, source: &str, rename: bool) {
    let old = title_of(rec);
    if title.is_empty() || title == old {
        return;
    }
    push_title(rec, title, source);
    // A goal that only repeated the old title follows the new one.
    if rec["goal"].as_str() == Some(old.as_str()) {
        rec["goal"] = json!(title);
        if let Ok(c) = config() {
            let _ = write_agent_files(&c, rec, space_dir);
        }
    }
    if let (true, Some(id)) = (rename, rec["herdr_space"].as_str()) {
        let _ = herdr("space.rename", &json!({ "id": id, "name": title }));
    }
}

/// A rename in Herdr is the person's own title.
fn sync_user_title(rec: &mut Value, spaces: &Map<String, Value>) -> bool {
    let id = rec["herdr_space"].as_str().unwrap_or("");
    let name = spaces.get(id).and_then(|s| s["name"].as_str()).unwrap_or("").to_string();
    if name.is_empty() || rec["title"].as_str() == Some(name.as_str()) {
        return false;
    }
    push_title(rec, &name, "user");
    true
}

/// Herdr's spaces by id, with their names now.
fn live_spaces() -> Map<String, Value> {
    let Ok(text) = run(&tool("herdr-app"), &["ls"], None) else { return Map::new() };
    let list: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let mut found = Map::new();
    for s in list["spaces"].as_array().into_iter().flatten() {
        if let Some(id) = s["id"].as_str() {
            found.insert(id.to_string(), s.clone());
        }
    }
    found
}

// ---------------------------------------------------------------- sessions of a space

/// The ids of the sessions that run in a Herdr space's terminals, from the sessions record.
fn live_session_ids(herdr_space: Option<&str>, spaces: &Map<String, Value>) -> Vec<String> {
    let Some(live) = herdr_space.and_then(|id| spaces.get(id)) else { return Vec::new() };
    let recorded = load_sessions();
    let tabs = live["tabs"].as_array().into_iter().flatten();
    let addrs = tabs.flat_map(|tab| tab["terminals"].as_array().into_iter().flatten()).filter_map(|t| t["terminal"].as_str());
    addrs.filter_map(|a| recorded.get(a).and_then(|e| e["session"].as_str()).map(String::from)).collect()
}

/// The folders of a space: its own, and the linked checkouts outside it.
fn space_roots(rec: &Value, space_dir: &Path) -> Vec<PathBuf> {
    let external = rec["checkouts"].as_array().into_iter().flatten().filter(|k| k["external"].as_bool().unwrap_or(false));
    std::iter::once(space_dir.to_path_buf()).chain(external.filter_map(|k| k["path"].as_str().map(PathBuf::from))).collect()
}

/// The agent sessions of a space: those that started in it or its checkouts, and those that
/// run in its Herdr terminals. Newest first.
fn space_sessions(agent: &dyn Agent, rec: &Value, space_dir: &Path, spaces: &Map<String, Value>) -> Result<Vec<Session>> {
    let roots = space_roots(rec, space_dir);
    let live = agent.sessions_by_id(&live_session_ids(rec["herdr_space"].as_str(), spaces))?;
    let live_ids: HashSet<String> = live.iter().map(|s| s.session.clone()).collect();
    let started = agent.sessions_in(&roots)?.into_iter().filter(|s| !live_ids.contains(&s.session));
    let all = live.into_iter().map(|s| (s, true)).chain(started.map(|s| (s, false)));
    let mut found = Vec::new();
    for (mut s, is_live) in all {
        let cwd = s.cwd.as_ref().map(PathBuf::from);
        let root = cwd.as_ref().and_then(|c| roots.iter().find(|r| c.starts_with(r)));
        if (root.is_none() && !is_live) || s.messages == 0 {
            continue;
        }
        s.place = Some(match (&cwd, root) {
            (Some(c), Some(r)) => {
                c.strip_prefix(r).ok().map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty()).unwrap_or_else(|| ".".into())
            }
            (Some(c), None) => c.to_string_lossy().into_owned(),
            _ => String::new(),
        });
        s.live = Some(is_live);
        found.push(s);
    }
    found.sort_by_key(|s| std::cmp::Reverse(s.at));
    Ok(found)
}

// ---------------------------------------------------------------- Smart rename

const SMART_PROMPT: &str = r#"You name a workspace in a developer's terminal app. One workspace can hold several
pieces of work at the same time: {agent} sessions, git branches with commits and
uncommitted files, and pull requests. The facts about this workspace are in the JSON below.

Return a JSON object with:
- "title": at most 48 characters, sentence case, no period, no emoji. It names all the
  active work of the workspace, not one session. When the pieces share a theme, name the
  theme. When there are two unrelated main pieces, join them with "and". Name what the work
  changes or delivers, not file names. Do not copy a session title unless it covers all the
  work. An open PR of the workspace is strong evidence of the main work, but sessions after
  it can add more. A long session moves through many topics: read all its prompts, from
  first to last, and name their common theme. Do not name only the last request.
  A session's "prs_mentioned" are PRs it showed or looked at: it did not always open them.
- "summary": one or two plain sentences: what the workspace does now and what is left.
  Use only the facts below. Do not name a PR, a branch, or a result that the facts do not show.
- "work": one item per separate piece of work, newest first: "title" (at most 40
  characters), "state" ("active" while a session works on it or it has uncommitted changes,
  "waiting" when it waits on a review or on the person, "done" when merged or finished), and
  "sessions" (the ids of its sessions).

Facts:
"#;

fn smart_schema() -> Value {
    json!({
        "type": "object", "required": ["title", "summary", "work"],
        "properties": {
            "title": { "type": "string" },
            "summary": { "type": "string" },
            "work": { "type": "array", "items": { "type": "object", "required": ["title", "state"], "properties": {
                "title": { "type": "string" }, "state": { "type": "string", "enum": ["active", "waiting", "done"] },
                "sessions": { "type": "array", "items": { "type": "string" } } } } },
        },
    })
}

/// A checkout's branch, its commits since the base, and its uncommitted files, in short.
fn git_facts(path: &Path, base: Option<&str>) -> Value {
    let g = |args: &[&str]| git_maybe(path, args, false).unwrap_or_default();
    let mut facts = Map::new();
    facts.insert("branch".into(), json!(g(&["rev-parse", "--abbrev-ref", "HEAD"])));
    if let Some(base) = base {
        let commits: Vec<String> = g(&["log", "--format=%s", "-n", "15", &format!("{base}..HEAD")]).lines().map(String::from).collect();
        facts.insert("base".into(), json!(base));
        facts.insert("commits".into(), json!(commits));
        facts.insert("diff".into(), json!(g(&["diff", "--shortstat", &format!("{base}...HEAD")])));
    }
    let mut args: Vec<&str> = READ.to_vec();
    args.extend(["status", "--porcelain"]);
    let status = git_maybe(path, &args, true).unwrap_or_default();
    let changed: Vec<String> = status.lines().filter(|l| !l.is_empty()).map(|l| l.get(3..).unwrap_or("").to_string()).collect();
    let mut shown: Vec<String> = changed.iter().take(20).cloned().collect();
    if changed.len() > 20 {
        shown.push(format!("and {} more", changed.len() - 20));
    }
    facts.insert("uncommitted".into(), json!(shown));
    Value::Object(facts)
}

/// The sessions for the Smart rename prompt: the newest 8.
fn session_facts(sessions: &[Session], now: i64) -> Value {
    let rows: Vec<Value> = sessions
        .iter()
        .take(8)
        .map(|s| {
            json!({
                "id": s.session, "title": s.title, "running": s.live.unwrap_or(false), "messages": s.messages,
                "minutes_since_active": (now - s.at) / 60000, "prompts_first_to_last": s.sample, "last_prompt": s.last,
                "prs_mentioned": s.prs.iter().map(|p| p.url.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Value::Array(rows)
}

/// JSON with an indent of one space: short, and easy for the agent to read.
fn pretty(v: &Value) -> String {
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, serde_json::ser::PrettyFormatter::with_indent(b" "));
    if v.serialize(&mut ser).is_err() {
        return v.to_string();
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Any JSON value as text: a string as it is, "" for null, else the JSON of the value.
fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Smart rename's answer from the agent: {title, summary, work}, cleaned.
fn ask_agent(agent: &dyn Agent, facts: &Value) -> Result<Value> {
    let prompt = SMART_PROMPT.replace("{agent}", &agent.label()) + &pretty(facts);
    let data = agent.smart_rename(&prompt, &smart_schema())?;
    clean_answer(&data)
}

fn clean_answer(data: &Value) -> Result<Value> {
    let title = text_of(&data["title"]);
    let title = title.trim().trim_end_matches('.');
    if title.is_empty() {
        fail!("The Smart rename agent gave no title: {}", data.to_string().chars().take(200).collect::<String>());
    }
    let work: Vec<Value> = data["work"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|w| w.is_object() && !matches!(&w["title"], Value::Null | Value::Bool(false)) && w["title"] != json!(""))
        .take(8)
        .cloned()
        .collect();
    Ok(json!({ "title": title.chars().take(60).collect::<String>(), "summary": text_of(&data["summary"]).trim(), "work": work }))
}

fn smart_metadata(rec: &mut Value, answer: &Value) {
    rec["summary"] = answer["summary"].clone();
    rec["work"] = answer["work"].clone();
    rec["smart_at"] = json!(now_secs());
}

/// A plain Herdr space (not made by Helm) has no record: its titles go here.
fn space_titles_file() -> PathBuf {
    state_dir().join("space-titles.json")
}

/// Names a plain Herdr space after its newest agent sessions, or `title`. The agent answers
/// first, with no lock. Then the titles file changes under its lock.
fn rename_plain(herdr_space: &str, title: Option<String>, spaces: &Map<String, Value>) -> Result<Value> {
    let Some(live) = spaces.get(herdr_space) else { fail!("No Herdr space {herdr_space}") };
    let live_name = live["name"].as_str().unwrap_or("");
    let answer = match title {
        Some(_) => None,
        None => Some(smart_plain(herdr_space, live, spaces)?),
    };
    let (title, source) = match (title, &answer) {
        (Some(t), _) => (t, "user"),
        (None, Some(a)) => (a["title"].as_str().unwrap_or("").to_string(), "smart"),
        (None, None) => (String::new(), "smart"),
    };
    let file = space_titles_file();
    let meta = locked(&file, || {
        let mut all_titles = match read_json(&file, json!({})) {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        let mut meta = all_titles
            .get(herdr_space)
            .filter(|m| m.is_object())
            .cloned()
            .unwrap_or_else(|| json!({ "titles": [{ "title": live_name, "source": "user", "at": null }] }));
        if let Some(a) = &answer {
            smart_metadata(&mut meta, a);
        }
        if title != live_name {
            let mut titles = meta["titles"].as_array().cloned().unwrap_or_default();
            titles.push(json!({ "title": title, "source": source, "at": now_secs() }));
            let extra = titles.len().saturating_sub(TITLES_KEPT);
            titles.drain(..extra);
            meta["titles"] = Value::Array(titles);
            herdr("space.rename", &json!({ "id": herdr_space, "name": title }))?;
        }
        meta["title"] = json!(title);
        meta["title_source"] = json!(source);
        // Forget the spaces that are closed.
        all_titles.retain(|k, _| spaces.contains_key(k));
        all_titles.insert(herdr_space.to_string(), meta.clone());
        write_json(&file, &Value::Object(all_titles))?;
        Ok(meta)
    })?;
    Ok(json!({ "title": title, "source": source, "new_prs": 0, "prs": [], "summary": meta["summary"], "work": meta["work"] }))
}

/// Smart rename of a plain space: the facts are its live sessions and the git repo of its folder.
fn smart_plain(herdr_space: &str, live: &Value, spaces: &Map<String, Value>) -> Result<Value> {
    let agent = agent::current_with("smart_rename")?;
    let ids = live_session_ids(Some(herdr_space), spaces);
    let mut sessions = agent.sessions_by_id(&ids)?;
    if sessions.is_empty() {
        fail!("No {} session runs in this space.", agent.label());
    }
    for s in &mut sessions {
        s.live = Some(true);
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.at));
    let mut facts = json!({ "current_title": live["name"], "folder": live["folder"], "sessions": session_facts(&sessions, now_ms()) });
    let folder = live["folder"].as_str().map(PathBuf::from).filter(|f| f.is_dir());
    if let Some(top) = folder.and_then(|f| git_maybe(&f, &["rev-parse", "--show-toplevel"], false)).filter(|t| !t.is_empty()) {
        facts["git"] = git_facts(Path::new(&top), None);
    }
    ask_agent(agent.as_ref(), &facts)
}

/// The folder of the project space whose Herdr space is `herdr_space`, if there is one.
fn space_of_herdr(c: &Value, herdr_space: &str) -> Option<String> {
    let dirs = std::fs::read_dir(tasks_root(c)).ok()?;
    dirs.flatten().map(|d| d.path()).find_map(|d| {
        let rec = read_json(&record_path(&d), Value::Null);
        (rec["herdr_space"] == herdr_space).then(|| d.file_name().map(|n| n.to_string_lossy().into_owned())).flatten()
    })
}

pub fn cmd_rename(c: &Ctx, a: &RenameArgs) -> Result<()> {
    let cfg = config()?;
    let title = a.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    let mut space = c.space.clone();
    if let Some(hs) = &a.herdr_space {
        // A project space by its Herdr id. Any other Herdr space is a plain one.
        match space_of_herdr(&cfg, hs) {
            Some(name) => space = Some(name),
            None => {
                let r = rename_plain(hs, title, &live_spaces())?;
                let by = title_source_text(r["source"].as_str().unwrap_or("user")).unwrap_or("you");
                out(&r, &format!("{} (named by {by})", or_dash(&r["title"])), c.json);
                return Ok(());
            }
        }
    }
    let (rec, space_dir) = load_space(&cfg, space.as_deref())?;
    let (rec, new) = match &title {
        Some(t) => update_space(&space_dir, |fresh| {
            set_title(fresh, &space_dir, t, "user", true);
            Ok(0)
        })?,
        None => smart_space(rec, &space_dir)?,
    };
    let source = title_source(&rec);
    let by = title_source_text(&source).unwrap_or("you");
    let data = json!({ "title": rec["title"], "source": source, "new_prs": new, "prs": prs_of(&rec), "summary": rec["summary"], "work": rec["work"] });
    out(&data, &format!("{}: {} (named by {by}; {new} new PRs)", or_dash(&rec["name"]), or_dash(&rec["title"])), c.json);
    Ok(())
}

/// Smart rename of a project space: links its PRs, then the agent names it after all its work.
/// The agent can take up to a minute, so it works on a copy of the record. Then only the fields
/// of Smart rename go into the record that is on disk at that time, under its lock. Returns the
/// new record and how many PRs are new.
fn smart_space(mut rec: Value, space_dir: &Path) -> Result<(Value, usize)> {
    let agent = agent::current_with("smart_rename")?;
    let spaces = live_spaces();
    sync_user_title(&mut rec, &spaces);
    let sessions = space_sessions(agent.as_ref(), &rec, space_dir, &spaces)?;
    let urls: Vec<String> = sessions.iter().flat_map(|s| s.prs.iter().map(|p| p.url.clone())).collect();
    let new = link_space_prs(&mut rec, &urls)?;
    let mut checkouts = Map::new();
    for k in rec["checkouts"].as_array().into_iter().flatten() {
        let path = Path::new(k["path"].as_str().unwrap_or(""));
        if path.exists() {
            checkouts.insert(k["folder"].as_str().unwrap_or("").to_string(), git_facts(path, k["base"].as_str()));
        }
    }
    let prs: Vec<Value> = prs_of(&rec)
        .iter()
        .map(|x| json!({ "repo": x["repo"], "number": x["number"], "title": x["title"], "state": x["state"] }))
        .collect();
    let facts = json!({
        "current_title": rec["title"], "set_by": title_source_text(&title_source(&rec)), "goal": rec["goal"], "project": rec["project"],
        "checkouts": checkouts, "prs": prs, "sessions": session_facts(&sessions, now_ms()),
    });
    let answer = ask_agent(agent.as_ref(), &facts)?;
    update_space(space_dir, |fresh| {
        sync_user_title(fresh, &spaces);
        apply_smart(fresh, &prs_of(&rec), &answer);
        set_title(fresh, space_dir, answer["title"].as_str().unwrap_or(""), "smart", true);
        Ok(new)
    })
}

/// Puts the fields of Smart rename into a fresh record: the PRs it found (with the PRs that
/// another command added in the meantime), and its summary, work and time. The title goes in
/// with `set_title`.
fn apply_smart(fresh: &mut Value, found_prs: &[Value], answer: &Value) {
    let mut prs = found_prs.to_vec();
    for p in prs_of(fresh) {
        let known = if p["key"].is_null() { prs.contains(&p) } else { prs.iter().any(|x| x["key"] == p["key"]) };
        if !known {
            prs.push(p);
        }
    }
    fresh["prs"] = Value::Array(prs);
    if let Some(o) = fresh.as_object_mut() {
        o.remove("pr");
    }
    smart_metadata(fresh, answer);
}

// ---------------------------------------------------------------- info and usage

/// A checkout's state, with its commits since the base.
fn checkout_info(rec: &Value, k: &Value) -> Value {
    let mut s = checkout_status(rec, k).unwrap_or_else(|_| k.clone());
    if s["missing"].as_bool() != Some(true) {
        let path = Path::new(k["path"].as_str().unwrap_or(""));
        let range = format!("{}..HEAD", k["base"].as_str().unwrap_or(""));
        let commits = git_maybe(path, &["rev-list", "--count", &range], false).and_then(|n| n.parse::<i64>().ok()).unwrap_or(0);
        s["commits"] = json!(commits);
    }
    s
}

/// The sessions of a space, or none when the agent kind is off or cannot list them.
fn sessions_or_none(rec: &Value, space_dir: &Path, spaces: &Map<String, Value>) -> Result<Vec<Session>> {
    match agent::current_with("sessions") {
        Ok(agent) => space_sessions(agent.as_ref(), rec, space_dir, spaces),
        Err(_) => Ok(Vec::new()),
    }
}

pub fn cmd_info(c: &Ctx) -> Result<()> {
    let (mut rec, space_dir) = load_space(&config()?, c.space.as_deref())?;
    let spaces = live_spaces();
    if sync_user_title(&mut rec.clone(), &spaces) {
        rec = update_space(&space_dir, |fresh| Ok(sync_user_title(fresh, &spaces)))?.0;
    }
    let ks = rec["checkouts"].as_array().cloned().unwrap_or_default();
    let (checkouts, sessions) = std::thread::scope(|scope| {
        let jobs: Vec<_> = ks.iter().map(|k| scope.spawn(|| checkout_info(&rec, k))).collect();
        let sessions = sessions_or_none(&rec, &space_dir, &spaces);
        let checkouts: Vec<Value> = jobs.into_iter().zip(&ks).map(|(j, k)| j.join().unwrap_or_else(|_| k.clone())).collect();
        (checkouts, sessions)
    });
    let sessions = sessions?;
    let live = rec["herdr_space"].as_str().and_then(|id| spaces.get(id)).cloned().unwrap_or(Value::Null);
    let mut notes: Vec<PathBuf> =
        std::fs::read_dir(space_dir.join(".notes")).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    notes.sort();
    let tabs: Vec<Value> = live["tabs"].as_array().into_iter().flatten().map(|t| t["name"].clone()).collect();

    let mut lines = vec![
        format!("{} ({}, {})", title_of(&rec), or_dash(&rec["name"]), or_dash(&rec["project"])),
        format!("Goal: {}", rec["goal"].as_str().unwrap_or("None")),
    ];
    let prs = prs_of(&rec);
    lines.extend(
        prs.iter().map(|x| format!("PR {} {}: {}", text_of(&x["key"]), x["state"].as_str().unwrap_or("None"), text_of(&x["title"]))),
    );
    lines.extend(checkouts.iter().map(|k| format!("{}: {} from {}", text_of(&k["folder"]), text_of(&k["branch"]), text_of(&k["base"]))));
    lines.extend(sessions.iter().map(|s| format!("Session {}: {}", s.session, s.title.as_deref().unwrap_or("None"))));

    let mut data = rec.as_object().cloned().unwrap_or_default();
    data.insert("path".into(), json!(space_dir.to_string_lossy()));
    data.insert("title_source".into(), json!(title_source(&rec)));
    data.insert("prs".into(), json!(prs));
    data.insert("checkouts".into(), json!(checkouts));
    data.insert("sessions".into(), serde_json::to_value(&sessions)?);
    data.insert("notes".into(), json!(notes.iter().map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>()));
    data.insert("tabs".into(), json!(tabs));
    out(&Value::Object(data), &lines.join("\n"), c.json);
    Ok(())
}

pub fn cmd_usage(c: &Ctx) -> Result<()> {
    let data = agent::current_with("usage")?.usage()?;
    let mut lines = vec![format!("Plan: {}", or_dash(&data["plan"]))];
    for l in data["limits"].as_array().into_iter().flatten() {
        let resets = l["resetsAt"].as_str().map(|r| format!(", resets at {r}")).unwrap_or_default();
        lines.push(format!("{}: {}%{resets}", or_dash(&l["label"]), text_of(&l["percent"])));
    }
    out(&data, &lines.join("\n"), c.json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::agent::SessionPr;

    fn session(id: &str, at: i64) -> Session {
        Session {
            session: id.into(),
            title: Some("T".into()),
            messages: 3,
            at,
            sample: vec!["a".into(), "b".into()],
            last: Some("b".into()),
            prs: vec![SessionPr { number: json!(4), url: "https://h/o/r/pull/4".into(), repo: "r".into() }],
            live: Some(true),
            ..Session::default()
        }
    }

    #[test]
    fn session_facts_give_the_newest_eight() {
        let sessions: Vec<Session> = (0..10).map(|i| session(&format!("s{i}"), 1_000_000 - i * 60_000)).collect();
        let facts = session_facts(&sessions, 1_000_000 + 120_000);
        let rows = facts.as_array().unwrap();
        assert_eq!(rows.len(), 8);
        assert_eq!(
            rows[1],
            json!({ "id": "s1", "title": "T", "running": true, "messages": 3, "minutes_since_active": 3,
                    "prompts_first_to_last": ["a", "b"], "last_prompt": "b", "prs_mentioned": ["https://h/o/r/pull/4"] })
        );
        let keys: Vec<&String> = rows[0].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["id", "title", "running", "messages", "minutes_since_active", "prompts_first_to_last", "last_prompt", "prs_mentioned"]
        );
    }

    #[test]
    fn set_title_keeps_a_history() {
        let dir = tempfile::tempdir().unwrap();
        // No herdr_space, so no rename in Herdr. The goal is not the old title, so CLAUDE.md stays.
        let mut rec = json!({ "name": "web-fix", "created": 100, "goal": "Something else" });
        set_title(&mut rec, dir.path(), "Fix the login", "user", false);
        let titles = rec["titles"].as_array().unwrap();
        assert_eq!(titles.len(), 2);
        assert_eq!(titles[0], json!({ "title": "web-fix", "source": "user", "at": 100 }));
        assert_eq!(titles[1]["title"], "Fix the login");
        assert_eq!(rec["title"], "Fix the login");
        assert_eq!(rec["title_source"], "user");

        // The same title, or none, changes nothing.
        set_title(&mut rec, dir.path(), "Fix the login", "smart", false);
        set_title(&mut rec, dir.path(), "", "smart", false);
        assert_eq!(rec["titles"].as_array().unwrap().len(), 2);
        assert_eq!(rec["title_source"], "user");

        // A goal that repeated the old title follows the new one.
        rec["goal"] = json!("Fix the login");
        set_title(&mut rec, dir.path(), "Login and signup", "smart", false);
        assert_eq!(rec["goal"], "Login and signup");

        // Only the last 20 titles stay.
        for i in 0..30 {
            set_title(&mut rec, dir.path(), &format!("Title {i}"), "smart", false);
        }
        let titles = rec["titles"].as_array().unwrap();
        assert_eq!(titles.len(), TITLES_KEPT);
        assert_eq!(titles.last().unwrap()["title"], "Title 29");
        assert_eq!(rec["title_source"], "smart");
    }

    #[test]
    fn sync_user_title_takes_the_herdr_name() {
        let mut rec = json!({ "name": "s", "title": "Old", "title_source": "smart", "herdr_space": "sp1" });
        let spaces: Map<String, Value> = serde_json::from_value(json!({ "sp1": { "name": "Mine" } })).unwrap();
        assert!(sync_user_title(&mut rec, &spaces));
        assert_eq!(rec["title"], "Mine");
        assert_eq!(rec["title_source"], "user");
        assert_eq!(rec["titles"][0]["title"], "Old");
        assert!(!sync_user_title(&mut rec, &spaces));
    }

    #[test]
    fn apply_smart_keeps_newer_changes() {
        // The record on disk got a checkout and a PR while the agent worked.
        let mut fresh = json!({ "name": "s", "checkouts": [{ "folder": "web" }, { "folder": "api" }],
            "prs": [{ "key": "o/web#1", "state": "MERGED" }, { "key": "o/api#2" }] });
        let found = vec![json!({ "key": "o/web#1", "state": "OPEN" }), json!({ "key": "o/web#3" })];
        let answer = json!({ "title": "T", "summary": "S", "work": [{ "title": "w" }] });
        apply_smart(&mut fresh, &found, &answer);
        assert_eq!(fresh["checkouts"].as_array().unwrap().len(), 2);
        let keys: Vec<&str> = fresh["prs"].as_array().unwrap().iter().filter_map(|p| p["key"].as_str()).collect();
        assert_eq!(keys, ["o/web#1", "o/web#3", "o/api#2"]);
        assert_eq!((fresh["summary"].as_str(), fresh["work"][0]["title"].as_str()), (Some("S"), Some("w")));
        assert!(fresh["smart_at"].as_i64().unwrap() > 0);
    }

    struct FakeAgent;

    impl Agent for FakeAgent {
        fn name(&self) -> String {
            "fake".into()
        }
        fn capabilities(&self) -> Vec<String> {
            vec!["resume".into()]
        }
        fn start_argv(&self) -> Result<Vec<String>> {
            Ok(vec!["agent".into()])
        }
        fn resume_argv(&self, argv: &[String], session: &str) -> Result<Vec<String>> {
            Ok([argv, &["--resume".to_string(), session.to_string()]].concat())
        }
    }

    #[test]
    fn resume_keeps_the_record_until_the_send_works() {
        let _env = crate::util::env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let hc = tmp.path().join("hc");
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{path}", tmp.path().display()));
        let terms: Map<String, Value> = serde_json::from_value(json!({
            "t1": { "foreground": { "argv": ["-zsh"] } },
            "t2": { "foreground": { "argv": ["vim"] } },
        }))
        .unwrap();
        let mut rec: Map<String, Value> = serde_json::from_value(json!({
            "t1": { "session": "s1", "cwd": "/w", "at": 5 },
            "t2": { "session": "s2", "cwd": "/w", "at": 5 },
            "t3": { "session": "s3", "cwd": "/w", "at": 5 },
        }))
        .unwrap();
        let write_hc = |code: i32| {
            std::fs::write(&hc, format!("#!/bin/sh\nexit {code}\n")).unwrap();
            let mut perms = std::fs::metadata(&hc).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
            std::fs::set_permissions(&hc, perms).unwrap();
        };
        // The send fails: t1 stays for the next try. t2 runs another program and t3 is gone.
        write_hc(1);
        let first = resume_ended(&FakeAgent, &mut rec, &terms, 10);
        let after_fail: Vec<String> = rec.keys().cloned().collect();
        write_hc(0);
        let second = resume_ended(&FakeAgent, &mut rec, &terms, 10);
        std::env::set_var("PATH", path);
        assert!(first.is_empty());
        assert_eq!(after_fail, ["t1"]);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0]["address"], "t1");
        assert!(rec.is_empty());
    }

    #[test]
    fn clean_answer_trims_the_title() {
        let a = clean_answer(&json!({ "title": " Fix login. ", "summary": " s ", "work": [{ "title": "w" }, { "x": 1 }, "no"] })).unwrap();
        assert_eq!(a, json!({ "title": "Fix login", "summary": "s", "work": [{ "title": "w" }] }));
        assert!(clean_answer(&json!({ "title": "  " })).is_err());
    }

    #[test]
    fn sh_quote_quotes_only_when_needed() {
        assert_eq!(sh_quote("claude"), "claude");
        assert_eq!(sh_quote("/a/b-c_d.e"), "/a/b-c_d.e");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_quote("it's here"), r#"'it'"'"'s here'"#);
    }

    #[test]
    fn agent_names() {
        assert!(valid_agent_name("api"));
        assert!(valid_agent_name("a1-b2"));
        assert!(!valid_agent_name("-a"));
        assert!(!valid_agent_name("toolongname"));
        assert!(!valid_agent_name("Api"));
    }
}
