//! The agent kind: the coding agent that runs in the terminals of a space.
//! Built in: claude-code. Others: an external program through the command connector.
//!
//! The trait holds all that is special to one agent: its command lines, the session of a
//! running process, its transcripts, the Smart rename call, and its usage limits. The generic
//! logic (the sessions record, titles, info, spawn and send through hc) is in
//! commands/agents.rs. docs/connectors-agent.md describes every method and its JSON.

pub mod claude_code;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// A PR that a session showed or opened.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SessionPr {
    pub number: Value,
    pub url: String,
    pub repo: String,
}

/// An agent session in short, as `info` shows it. The field order is the JSON key order.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Session {
    pub session: String,
    /// The title the agent gave it, or the person's own.
    pub title: Option<String>,
    /// True when the person named it.
    pub named: bool,
    /// The folder it started in.
    pub cwd: Option<String>,
    pub prs: Vec<SessionPr>,
    /// How many prompts the person wrote.
    pub messages: u64,
    /// The first prompt.
    pub prompt: Option<String>,
    /// The last prompt.
    pub last: Option<String>,
    /// When it was last active, in ms.
    pub at: i64,
    /// Up to 12 prompts, spread from the first to the last.
    pub sample: Vec<String>,
    /// Set by the generic code: its folder relative to the space.
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// Set by the generic code: true when it runs in a terminal of the space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<bool>,
}

/// The session of a running agent process.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Running {
    pub session: String,
    pub cwd: Option<String>,
}

/// One coding agent. Only `name`, `capabilities` and `start_argv` are required: the defaults
/// of the others say "cannot", and `capabilities` tells the commands and the mods what works.
pub trait Agent {
    /// The connector's name, e.g. "claude-code".
    fn name(&self) -> String;
    /// The name for a person, in messages, e.g. "Claude Code".
    fn label(&self) -> String {
        self.name()
    }
    /// The icon of the agent for the bar: {"path": an SVG file, relative to the Herdr home, where
    /// Herdr loads icons from, "color": a color such as "#d97757" or null}. None: a plain meter.
    fn icon(&self) -> Option<serde_json::Value> {
        None
    }
    /// What it can do: "start", "sessions", "resume", "restart", "smart_rename", "usage".
    fn capabilities(&self) -> Vec<String>;
    /// The command line that starts a new agent in a terminal, e.g. ["claude"].
    fn start_argv(&self) -> Result<Vec<String>>;
    /// The command line that starts a new agent that may also work in these folders (the
    /// linked checkouts outside the space folder).
    fn start_argv_in(&self, _dirs: &[String]) -> Result<Vec<String>> {
        self.start_argv()
    }
    /// The program names of this agent, as a terminal's foreground process shows them.
    fn programs(&self) -> Vec<String> {
        Vec::new()
    }
    /// The session of the running agent process `pid`, or None when it has none yet.
    fn running_session(&self, _pid: i64) -> Result<Option<Running>> {
        Ok(None)
    }
    /// The command line `argv` (of a running agent, or start_argv) changed to resume `session`.
    fn resume_argv(&self, _argv: &[String], _session: &str) -> Result<Vec<String>> {
        Err(anyhow!("the agent connector '{}' cannot resume a session", self.name()))
    }
    /// The sessions that started in these folders or under them. The generic code checks the
    /// folder again, so a connector may return more.
    fn sessions_in(&self, _roots: &[PathBuf]) -> Result<Vec<Session>> {
        Ok(Vec::new())
    }
    /// The sessions with these ids, in the same order. Unknown ids are left out.
    fn sessions_by_id(&self, _ids: &[String]) -> Result<Vec<Session>> {
        Ok(Vec::new())
    }
    /// Smart rename: gives `prompt` to the agent and returns its JSON answer, an object that
    /// matches `schema` ({title, summary, work}).
    fn smart_rename(&self, _prompt: &str, _schema: &Value) -> Result<Value> {
        Err(anyhow!("the agent connector '{}' cannot do Smart rename", self.name()))
    }
    /// The command that Smart rename runs when `agent_settings.smart_command` is empty, to show
    /// in Helm Settings. None when the connector has no such command.
    fn smart_default(&self) -> Option<String> {
        None
    }
    /// The usage limits: {at, plan, limits: [{kind, label, short, percent, resetsAt, seconds}]}.
    fn usage(&self) -> Result<Value> {
        Err(anyhow!("the agent connector '{}' cannot read usage limits", self.name()))
    }
}

/// The chosen agent connector: projects.json "connectors.agent" (default "claude-code").
/// The name "none" turns the kind off.
pub fn current() -> Result<Box<dyn Agent>> {
    match super::choice("agent", "claude-code") {
        super::Choice::Command(program) => Ok(Box::new(super::command::Command::new("agent", program))),
        super::Choice::Builtin(name) => match name.as_str() {
            "claude-code" => Ok(Box::new(claude_code::ClaudeCode::new())),
            "none" => Err(anyhow!("the agent connector is off (projects.json connectors.agent is \"none\")")),
            other => Err(anyhow!("unknown agent connector '{other}' (built in: claude-code)")),
        },
    }
}

/// The chosen agent connector, when it can do `capability`.
pub fn current_with(capability: &str) -> Result<Box<dyn Agent>> {
    let agent = current()?;
    if !agent.capabilities().iter().any(|c| c == capability) {
        return Err(anyhow!("the agent connector '{}' cannot do '{capability}'", agent.name()));
    }
    Ok(agent)
}

/// The command line of a new agent in a space. The linked (external) checkouts live outside
/// the space folder, so the agent gets them as extra folders.
pub fn space_argv(rec: &Value) -> Result<Vec<String>> {
    let dirs: Vec<String> = rec["checkouts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|k| k["external"].as_bool().unwrap_or(false))
        .filter_map(|k| k["path"].as_str().map(String::from))
        .collect();
    current_with("start")?.start_argv_in(&dirs)
}

pub fn describe() -> Value {
    match current() {
        // label, start and programs: the mods name the agent, start it in a plain space, and know
        // its tiles.
        // icon: the usage meter shows the agent's own icon.
        Ok(a) => {
            serde_json::json!({ "name": a.name(), "label": a.label(), "capabilities": a.capabilities(), "start": a.start_argv().unwrap_or_default(), "programs": a.programs(), "icon": a.icon() })
        }
        Err(_) => super::none(),
    }
}
