//! The claude-code agent connector. It reads what Claude Code keeps under ~/.claude:
//!
//!   sessions/<pid>.json             the session of each running process (not documented: if
//!                                   it changes, no session is found and nothing is recorded)
//!   projects/<folder key>/<id>.jsonl  the transcript of each session, by its start folder
//!
//! Smart rename runs the configured command (default `claude -p --model haiku`). The usage
//! limits come from the endpoint behind Claude Code's /usage, and only on macOS when the person
//! turns them on (agent_settings.usage), because they need the sign-in token from the keychain.

use super::{Running, Session, SessionPr};
use crate::util::{exec_with, home, now_ms, read_json};
use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SMART_COMMAND: &str = "claude -p --model haiku";
const SMART_TIMEOUT: Duration = Duration::from_secs(60);
/// The keychain read and the request of `usage` (curl itself stops after 20 seconds).
const USAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// The length of each usage window in seconds, by its kind. A kind that is not here is not shown.
const WINDOWS: [(&str, i64); 3] = [("session", 5 * 3600), ("weekly_all", 7 * 86400), ("weekly_scoped", 7 * 86400)];

/// The token stays in the shell: curl reads the header from stdin, so it is never in an
/// argument list (ps) or in this program. The first line is the plan, the rest the usage.
const USAGE_SCRIPT: &str = r#"
c=$(security find-generic-password -s "Claude Code-credentials" -w) || { echo "No Claude Code sign-in in the keychain" >&2; exit 3; }
token=$(printf %s "$c" | sed -nE 's/.*"accessToken":"([^"]*)".*/\1/p')
[ -n "$token" ] || { echo "No access token in the Claude Code sign-in" >&2; exit 4; }
printf '%s %s\n' "$(printf %s "$c" | sed -nE 's/.*"subscriptionType":"([^"]*)".*/\1/p')" "$(printf %s "$c" | sed -nE 's/.*"rateLimitTier":"([^"]*)".*/\1/p')"
printf 'Authorization: Bearer %s\n' "$token" | curl -sS -f --max-time 20 -H @- -H "anthropic-beta: oauth-2025-04-20" https://api.anthropic.com/api/oauth/usage
"#;

pub struct ClaudeCode;

impl ClaudeCode {
    pub fn new() -> Self {
        ClaudeCode
    }
}

fn claude_dir() -> PathBuf {
    home().join(".claude")
}

/// projects.json "agent_settings".
fn settings() -> Value {
    crate::connectors::settings("agent")
}

/// The usage limits work only on macOS, where `security` reads the sign-in from the keychain,
/// and only when agent_settings.usage is true.
fn usage_on() -> bool {
    cfg!(target_os = "macos") && settings()["usage"].as_bool().unwrap_or(false)
}

/// The Smart rename command: agent_settings.smart_command, else the older
/// smart_rename.command, else the default.
fn smart_command() -> String {
    let from = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    if let Some(cmd) = from(&settings()["smart_command"]) {
        return cmd;
    }
    let config = read_json(&crate::paths::config_file(), json!({}));
    from(&config["smart_rename"]["command"]).unwrap_or_else(|| SMART_COMMAND.to_string())
}

impl super::Agent for ClaudeCode {
    fn name(&self) -> String {
        "claude-code".into()
    }
    fn label(&self) -> String {
        "Claude Code".into()
    }
    /// Herdr ships program icons, Claude's among them: Helm uses it and has no copy of its own.
    fn icon(&self) -> Option<Value> {
        let path = "icons/claude.svg";
        crate::paths::app_home().join(path).is_file().then(|| json!({ "path": path, "color": "#d97757" }))
    }
    fn smart_default(&self) -> Option<String> {
        Some(SMART_COMMAND.to_string())
    }
    fn capabilities(&self) -> Vec<String> {
        let mut caps: Vec<String> = ["start", "sessions", "resume", "restart", "smart_rename"].map(String::from).to_vec();
        if usage_on() {
            caps.push("usage".into());
        }
        caps
    }
    fn start_argv(&self) -> Result<Vec<String>> {
        Ok(vec!["claude".into()])
    }
    fn start_argv_in(&self, dirs: &[String]) -> Result<Vec<String>> {
        let mut argv = self.start_argv()?;
        for d in dirs {
            argv.extend(["--add-dir".to_string(), d.clone()]);
        }
        Ok(argv)
    }
    fn programs(&self) -> Vec<String> {
        vec!["claude".into()]
    }
    fn running_session(&self, pid: i64) -> Result<Option<Running>> {
        let info = read_json(&claude_dir().join("sessions").join(format!("{pid}.json")), Value::Null);
        Ok(info["sessionId"]
            .as_str()
            .map(|id| Running { session: id.to_string(), cwd: info["cwd"].as_str().filter(|c| !c.is_empty()).map(String::from) }))
    }
    fn resume_argv(&self, argv: &[String], session: &str) -> Result<Vec<String>> {
        let mut out = without_session(argv);
        out.extend(["--resume".to_string(), session.to_string()]);
        Ok(out)
    }
    fn sessions_in(&self, roots: &[PathBuf]) -> Result<Vec<Session>> {
        let keys: Vec<String> = roots.iter().map(|r| folder_key(r)).collect();
        let Ok(dirs) = std::fs::read_dir(claude_dir().join("projects")) else { return Ok(Vec::new()) };
        let mut found = Vec::new();
        for d in dirs.flatten() {
            let name = d.file_name().to_string_lossy().into_owned();
            // "/" and "-" both become "-", so the folder name only narrows the search.
            if keys.iter().any(|k| name == *k || name.starts_with(&format!("{k}-"))) {
                for f in transcripts(&d.path()) {
                    found.push(read_session(&f)?);
                }
            }
        }
        Ok(found)
    }
    fn sessions_by_id(&self, ids: &[String]) -> Result<Vec<Session>> {
        let projects = claude_dir().join("projects");
        let dirs: Vec<PathBuf> = std::fs::read_dir(&projects).map(|r| r.flatten().map(|d| d.path()).collect()).unwrap_or_default();
        let mut found = Vec::new();
        for id in ids {
            // An imported session has its original and its copy: the copy is the one in use.
            let newest = dirs.iter().map(|d| d.join(format!("{id}.jsonl"))).filter(|f| f.is_file()).max_by_key(|f| mtime_ms(f));
            if let Some(f) = newest {
                found.push(read_session(&f)?);
            }
        }
        Ok(found)
    }
    fn smart_rename(&self, prompt: &str, schema: &Value) -> Result<Value> {
        let line = smart_command();
        let base = shlex::split(&line)
            .filter(|a| !a.is_empty())
            .ok_or_else(|| anyhow!("The Smart rename command is not a valid command line: {line}"))?;
        let (argv, prompt) = smart_argv(base, prompt, schema);
        let cwd = crate::paths::state_dir().join("smart");
        std::fs::create_dir_all(&cwd)?;
        let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
        let out = crate::util::exec_with(&argv[0], &args, Some(&cwd), Some(&prompt), Some(SMART_TIMEOUT))
            .map_err(|e| anyhow!("The Smart rename agent did not answer: {e}"))?;
        if !out.ok() {
            let why = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
            return Err(anyhow!("The Smart rename agent failed: {}", last_chars(why, 300)));
        }
        let text = out.stdout.trim();
        let data = parse_answer(text);
        if data["title"].as_str().is_none_or(|t| t.trim().is_empty()) && !data["title"].is_number() {
            return Err(anyhow!("The Smart rename agent gave no title: {}", text.chars().take(200).collect::<String>()));
        }
        Ok(data)
    }
    fn usage(&self) -> Result<Value> {
        if !cfg!(target_os = "macos") {
            return Err(anyhow!("usage needs the macOS keychain: claude-code reads usage limits only on macOS"));
        }
        if !usage_on() {
            return Err(anyhow!("usage is off: set agent_settings.usage to true"));
        }
        let out = exec_with("sh", &["-c", USAGE_SCRIPT], None, None, Some(USAGE_TIMEOUT))?;
        if !out.ok() {
            let why = out.stderr.trim().lines().last().map(String::from).unwrap_or_else(|| format!("exit {}", out.code));
            return Err(anyhow!("{why}"));
        }
        let (first, body) = out.stdout.split_once('\n').unwrap_or((out.stdout.as_str(), ""));
        let reply: Value = serde_json::from_str(body).map_err(|e| anyhow!("the usage reply is not JSON: {e}"))?;
        Ok(usage_from(first, &reply, now_ms()))
    }
}

/// The flags of a Claude command without those that pick a session: --resume/-r and
/// --continue/-c, and the value of --resume when it has one.
pub fn without_session(argv: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for (i, x) in argv.iter().enumerate() {
        if skip {
            skip = false;
            continue;
        }
        if x == "-c" || x == "--continue" || x.starts_with("--resume=") {
            continue;
        }
        if x == "-r" || x == "--resume" {
            // The value is the next item when it does not start with "-".
            skip = argv.get(i + 1).is_some_and(|next| !next.starts_with('-'));
            continue;
        }
        out.push(x.clone());
    }
    out
}

/// Claude Code's folder name for a start folder: each character that is not a letter or a
/// digit becomes "-".
fn folder_key(path: &Path) -> String {
    path.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn transcripts(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl") && p.is_file()).collect()
}

fn mtime_ms(f: &Path) -> i64 {
    std::fs::metadata(f)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

fn first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn last_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

/// A Claude transcript in short: the title Claude gave it (or the person's own), the PRs it
/// opened, its prompts, how many messages, and when it was last active.
pub fn read_session(f: &Path) -> Result<Session> {
    let mut s = Session {
        session: f.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(),
        at: mtime_ms(f),
        ..Session::default()
    };
    let mut prompts: Vec<String> = Vec::new();
    let mut reader = BufReader::new(std::fs::File::open(f)?);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        read_line(&line, &mut s, &mut prompts);
    }
    s.prompt = prompts.first().map(|p| first_chars(p, 300));
    s.sample = spread(prompts, 12);
    Ok(s)
}

/// Takes in one transcript line. Only the records used here are parsed: transcripts grow to
/// many MB.
fn read_line(line: &str, s: &mut Session, prompts: &mut Vec<String>) {
    let parse = || serde_json::from_str::<Value>(line).unwrap_or(Value::Null);
    let text = |v: &Value| v.as_str().filter(|t| !t.is_empty()).map(String::from);
    let user = line.contains(r#""type":"user""#) && !line.contains(r#""tool_use_id""#);
    if user {
        s.messages += 1;
    }
    if s.cwd.is_none() && line.contains(r#""cwd":""#) {
        s.cwd = text(&parse()["cwd"]);
    }
    if line.contains(r#""type":"custom-title""#) {
        s.title = text(&parse()["customTitle"]).or(s.title.take());
        s.named = true;
    } else if line.contains(r#""type":"ai-title""#) {
        if !s.named {
            s.title = text(&parse()["aiTitle"]).or(s.title.take());
        }
    } else if line.contains(r#""type":"last-prompt""#) {
        let last = first_chars(parse()["lastPrompt"].as_str().unwrap_or("").trim(), 300);
        if !last.is_empty() {
            s.last = Some(last);
        }
    } else if line.contains(r#""type":"pr-link""#) {
        let j = parse();
        if let Some(url) = text(&j["prUrl"]) {
            if !s.prs.iter().any(|p| p.url == url) {
                let repo = j["prRepository"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
                s.prs.push(SessionPr { number: j["prNumber"].clone(), url, repo });
            }
        }
    } else if user {
        if let Some(content) = parse()["message"]["content"].as_str() {
            if !content.trim().is_empty() && !content.starts_with('<') {
                prompts.push(first_chars(content.trim(), 160));
            }
        }
    }
}

/// At most `n` items, spread evenly from the first to the last: a long session moves through
/// many topics.
fn spread(items: Vec<String>, n: usize) -> Vec<String> {
    let len = items.len();
    if len <= n {
        return items;
    }
    // round(i * (len - 1) / (n - 1)) in whole numbers. With n = 12 it is never exactly .5.
    (0..n).map(|i| items[(2 * i * (len - 1) + (n - 1)) / (2 * (n - 1))].clone()).collect()
}

/// The command line and the prompt for Smart rename. Only claude gets the flags for one
/// answer: no tools, no session left behind in a space's transcripts, and the JSON schema.
fn smart_argv(mut argv: Vec<String>, prompt: &str, schema: &Value) -> (Vec<String>, String) {
    let is_claude = Path::new(&argv[0]).file_name().is_some_and(|n| n == "claude");
    if !is_claude {
        return (argv, format!("{prompt}\n\nAnswer with only the JSON object."));
    }
    if !argv.iter().any(|a| a == "-p" || a == "--print") {
        argv.push("-p".into());
    }
    let flags = [
        "--output-format",
        "json",
        "--no-session-persistence",
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--json-schema",
    ];
    argv.extend(flags.map(String::from));
    argv.push(schema.to_string());
    (argv, prompt.to_string())
}

/// The answer object in the agent's output: claude's structured output, else the JSON text,
/// else the first {...} in the text. Null when there is none.
fn parse_answer(text: &str) -> Value {
    let from_text = || {
        let re = Regex::new(r"(?s)\{.*\}").expect("valid regex");
        re.find(text).and_then(|m| serde_json::from_str(m.as_str()).ok()).unwrap_or(Value::Null)
    };
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(o)) if o.contains_key("structured_output") => {
            let so = &o["structured_output"];
            if !so.is_null() && so != &json!({}) && so != &json!(false) {
                return so.clone();
            }
            let result = o.get("result").and_then(Value::as_str).filter(|r| !r.is_empty()).unwrap_or("{}");
            serde_json::from_str(result).unwrap_or_else(|_| from_text())
        }
        Ok(v) => v,
        Err(_) => from_text(),
    }
}

/// The usage JSON from the plan line ("<subscription> <tier>") and the endpoint's reply, in
/// the shape that the helm-jobs mod builds.
///
/// The endpoint is not documented, so its reply shape is not sure. This reads two shapes, and
/// gives an empty list for anything else: a missing or unknown field is never an error.
///   {"limits": [{"kind", "percent", "resets_at", "scope"}]}       (the shape Helm first used)
///   {"five_hour": {"utilization", "resets_at"}, "seven_day": {...}, "seven_day_<model>": {...}}
pub fn usage_from(plan_line: &str, reply: &Value, at: i64) -> Value {
    let mut parts = plan_line.trim_end_matches('\r').splitn(2, ' ');
    let sub = parts.next().unwrap_or("").to_string();
    let tier = parts.next().unwrap_or("");
    let max = Regex::new(r"max_(\d+x)").expect("valid regex").captures(tier).map(|c| format!("max {}", &c[1]));
    let plan: Vec<String> = [Some(sub), max].into_iter().flatten().filter(|s| !s.is_empty()).collect();
    let mut limits: Vec<Value> = reply["limits"].as_array().into_iter().flatten().filter_map(limit_of_list).collect();
    if limits.is_empty() {
        limits = limits_of_windows(reply);
    }
    json!({ "at": at, "plan": plan.join(" · "), "limits": limits })
}

/// One limit in the helm-jobs shape. None for a kind that Helm does not show.
fn limit(kind: &str, model: Option<&str>, percent: &Value, resets_at: &Value) -> Option<Value> {
    let seconds = WINDOWS.iter().find(|(k, _)| *k == kind)?.1;
    percent.as_f64()?;
    let (label, short) = match kind {
        "session" => ("5 hours".to_string(), "5h"),
        "weekly_all" => ("Week".to_string(), "wk"),
        _ => (format!("{} week", model.unwrap_or("Model")), "wk"),
    };
    let resets_at = if resets_at.is_string() { resets_at.clone() } else { Value::Null };
    Some(json!({ "kind": kind, "label": label, "short": short, "percent": percent, "resetsAt": resets_at, "seconds": seconds }))
}

/// A limit of the {"limits": [...]} shape.
fn limit_of_list(l: &Value) -> Option<Value> {
    limit(l["kind"].as_str()?, l["scope"]["model"]["display_name"].as_str(), &l["percent"], &l["resets_at"])
}

/// The limits of the {"five_hour": {...}, "seven_day": {...}} shape. A "seven_day_<model>"
/// key is the weekly limit of one model.
fn limits_of_windows(reply: &Value) -> Vec<Value> {
    let Some(o) = reply.as_object() else { return Vec::new() };
    o.iter()
        .filter_map(|(key, w)| {
            let (kind, model) = match key.as_str() {
                "five_hour" => ("session", None),
                "seven_day" => ("weekly_all", None),
                other => {
                    let model = other.strip_prefix("seven_day_")?;
                    // Not a model: the limit of the OAuth apps, extra usage, and the like.
                    if model.contains("oauth") || model.contains("app") {
                        return None;
                    }
                    ("weekly_scoped", Some(capitalized(model)))
                }
            };
            limit(kind, model.as_deref(), &w["utilization"], &w["resets_at"])
        })
        .collect()
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn without_session_drops_the_session_flags() {
        let argv = strings(&["claude", "--resume", "abc", "--model", "opus", "-c", "--add-dir", "/x"]);
        assert_eq!(without_session(&argv), strings(&["claude", "--model", "opus", "--add-dir", "/x"]));
        let argv = strings(&["claude", "-r", "--verbose", "--resume=abc", "--continue"]);
        assert_eq!(without_session(&argv), strings(&["claude", "--verbose"]));
        assert_eq!(without_session(&strings(&["claude", "--resume"])), strings(&["claude"]));
    }

    #[test]
    fn resume_argv_adds_the_session() {
        use crate::connectors::agent::Agent;
        let argv = ClaudeCode.resume_argv(&strings(&["claude", "--resume", "old", "--model", "opus"]), "new").unwrap();
        assert_eq!(argv, strings(&["claude", "--model", "opus", "--resume", "new"]));
    }

    #[test]
    fn folder_key_is_claudes() {
        assert_eq!(folder_key(Path::new("/Users/me/herdr/tasks/a_b.c")), "-Users-me-herdr-tasks-a-b-c");
    }

    #[test]
    fn read_session_reads_a_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sess-1.jsonl");
        let lines = [
            r#"{"type":"user","cwd":"/w/space","message":{"content":"  Fix the login bug  "}}"#,
            r#"{"type":"assistant","message":{"content":"ok"}}"#,
            r#"{"type":"user","message":{"content":[{"tool_use_id":"x","type":"tool_result"}]}}"#,
            r#"{"type":"ai-title","aiTitle":"Login bug"}"#,
            r#"{"type":"user","cwd":"/w/other","message":{"content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"pr-link","prNumber":12,"prUrl":"https://github.com/o/web/pull/12","prRepository":"o/web"}"#,
            r#"{"type":"pr-link","prNumber":12,"prUrl":"https://github.com/o/web/pull/12","prRepository":"o/web"}"#,
            r#"{"type":"custom-title","customTitle":"My name"}"#,
            r#"{"type":"ai-title","aiTitle":"Later AI title"}"#,
            r#"{"type":"last-prompt","lastPrompt":"  ship it "}"#,
            "not json at all \u{fffd}",
            r#"{"type":"user","message":{"content":"Add tests"}}"#,
        ];
        std::fs::write(&f, lines.join("\n") + "\n").unwrap();
        let s = read_session(&f).unwrap();
        assert_eq!(s.session, "sess-1");
        assert_eq!(s.title.as_deref(), Some("My name"));
        assert!(s.named);
        assert_eq!(s.cwd.as_deref(), Some("/w/space"));
        assert_eq!(s.messages, 3);
        assert_eq!(s.prompt.as_deref(), Some("Fix the login bug"));
        assert_eq!(s.last.as_deref(), Some("ship it"));
        assert_eq!(s.sample, strings(&["Fix the login bug", "Add tests"]));
        assert_eq!(s.prs, vec![SessionPr { number: json!(12), url: "https://github.com/o/web/pull/12".into(), repo: "web".into() }]);
        assert!(s.at > 0);
        let keys: Vec<String> = serde_json::to_value(&s).unwrap().as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, strings(&["session", "title", "named", "cwd", "prs", "messages", "prompt", "last", "at", "sample"]));
    }

    #[test]
    fn spread_picks_evenly_from_first_to_last() {
        let items: Vec<String> = (0..30).map(|i| i.to_string()).collect();
        // Item round(i * 29 / 11) for i in 0..12: the first, the last, and ten between them.
        let want = strings(&["0", "3", "5", "8", "11", "13", "16", "18", "21", "24", "26", "29"]);
        assert_eq!(spread(items, 12), want);
    }

    #[test]
    fn smart_argv_adds_claude_flags_only_to_claude() {
        let schema = json!({"type": "object"});
        let (argv, prompt) = smart_argv(strings(&["claude", "--model", "haiku"]), "P", &schema);
        assert_eq!(argv[..4], strings(&["claude", "--model", "haiku", "-p"])[..]);
        assert_eq!(argv.last().unwrap(), r#"{"type":"object"}"#);
        assert_eq!(prompt, "P");
        let (argv, prompt) = smart_argv(strings(&["llm", "-m", "x"]), "P", &schema);
        assert_eq!(argv, strings(&["llm", "-m", "x"]));
        assert!(prompt.ends_with("Answer with only the JSON object."));
    }

    #[test]
    fn parse_answer_finds_the_object() {
        assert_eq!(parse_answer(r#"{"structured_output":{"title":"A"}}"#), json!({"title": "A"}));
        assert_eq!(parse_answer(r#"{"structured_output":null,"result":"{\"title\":\"B\"}"}"#), json!({"title": "B"}));
        assert_eq!(parse_answer("Sure: {\"title\": \"C\"} done"), json!({"title": "C"}));
        assert_eq!(parse_answer("nothing"), Value::Null);
    }

    // The usage endpoint is not documented, and these tests never call it or read the keychain.
    // So each sample reply below is an ASSUMED shape, not a copy of a real reply.

    #[test]
    fn usage_maps_an_assumed_limits_reply() {
        let reply = json!({ "limits": [
            { "kind": "session", "percent": 23.5, "resets_at": "2026-11-01T15:00:00Z" },
            { "kind": "weekly_all", "percent": 40, "resets_at": "2026-11-05T00:00:00Z" },
            { "kind": "weekly_scoped", "percent": 10, "scope": { "model": { "display_name": "Opus" } } },
            { "kind": "weekly_scoped", "percent": 5 },
            { "kind": "other", "percent": 1 },
            { "kind": "session", "percent": null },
        ]});
        let got = usage_from("max default_claude_max_20x", &reply, 1000);
        assert_eq!(
            got,
            json!({
                "at": 1000,
                "plan": "max · max 20x",
                "limits": [
                    { "kind": "session", "label": "5 hours", "short": "5h", "percent": 23.5, "resetsAt": "2026-11-01T15:00:00Z", "seconds": 18000 },
                    { "kind": "weekly_all", "label": "Week", "short": "wk", "percent": 40, "resetsAt": "2026-11-05T00:00:00Z", "seconds": 604800 },
                    { "kind": "weekly_scoped", "label": "Opus week", "short": "wk", "percent": 10, "resetsAt": null, "seconds": 604800 },
                    { "kind": "weekly_scoped", "label": "Model week", "short": "wk", "percent": 5, "resetsAt": null, "seconds": 604800 },
                ],
            })
        );
    }

    #[test]
    fn usage_maps_an_assumed_windows_reply() {
        let reply = json!({
            "five_hour": { "utilization": 12.0, "resets_at": "2026-11-01T15:00:00Z" },
            "seven_day": { "utilization": 30, "resets_at": null },
            "seven_day_opus": { "utilization": 4 },
            "seven_day_oauth_apps": null,
            "extra_usage": { "is_enabled": false },
        });
        let got = usage_from("pro ", &reply, 7);
        assert_eq!(got["plan"], "pro");
        assert_eq!(
            got["limits"],
            json!([
                { "kind": "session", "label": "5 hours", "short": "5h", "percent": 12.0, "resetsAt": "2026-11-01T15:00:00Z", "seconds": 18000 },
                { "kind": "weekly_all", "label": "Week", "short": "wk", "percent": 30, "resetsAt": null, "seconds": 604800 },
                { "kind": "weekly_scoped", "label": "Opus week", "short": "wk", "percent": 4, "resetsAt": null, "seconds": 604800 },
            ])
        );
    }

    #[test]
    fn usage_of_an_unknown_reply_is_empty() {
        let empty = |reply: Value| usage_from(" ", &reply, 5);
        assert_eq!(empty(json!({})), json!({ "at": 5, "plan": "", "limits": [] }));
        for reply in [
            json!(null),
            json!([1, 2]),
            json!("text"),
            json!({ "limits": "x" }),
            json!({ "limits": [{ "percent": 3 }] }),
            json!({ "five_hour": 3 }),
        ] {
            assert_eq!(empty(reply.clone())["limits"], json!([]), "{reply}");
        }
    }

    #[test]
    fn usage_is_a_capability_only_on_macos() {
        let _env = crate::util::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("projects.json");
        std::fs::write(&config, r#"{"agent_settings": {"usage": true}}"#).unwrap();
        std::env::set_var("HERDR_PROJECTS", &config);
        let caps = super::super::Agent::capabilities(&ClaudeCode::new());
        assert_eq!(caps.iter().any(|c| c == "usage"), cfg!(target_os = "macos"));
    }
}
