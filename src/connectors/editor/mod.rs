//! The editor kind: where a file, a diff, or a folder opens. Built in: vscode. Others: an
//! external program through the command form (ops "open", "diff", "window").

mod vscode;

use anyhow::Result;
use serde_json::Value;

pub trait Editor {
    /// The connector's name, e.g. "vscode".
    fn name(&self) -> String;
    /// What it can do: "open", "diff", "window".
    fn capabilities(&self) -> Vec<String>;
    /// Opens a file.
    fn open(&self, path: &str) -> Result<()>;
    /// Shows the diff of two files. `reuse`: in the window that opened last (after `window`).
    fn diff(&self, old: &str, new: &str, reuse: bool) -> Result<()>;
    /// Opens a folder in a new window.
    fn window(&self, folder: &str) -> Result<()>;
}

/// The chosen editor connector: projects.json "connectors.editor" (default "vscode").
pub fn current() -> Result<Box<dyn Editor>> {
    match super::choice("editor", "vscode") {
        super::Choice::Command(program) => Ok(Box::new(super::command::Command::new("editor", program))),
        super::Choice::Builtin(name) => match name.as_str() {
            "vscode" => Ok(Box::new(vscode::VsCode::new())),
            "none" => anyhow::bail!("the editor connector is off (connectors.editor is \"none\")"),
            other => anyhow::bail!("unknown editor connector '{other}' (built in: vscode)"),
        },
    }
}

/// The chosen editor connector, when it can do `capability`.
pub fn current_with(capability: &str) -> Result<Box<dyn Editor>> {
    let editor = current()?;
    if !super::has(&editor.capabilities(), capability) {
        anyhow::bail!("the editor connector '{}' cannot do '{capability}'", editor.name());
    }
    Ok(editor)
}

pub fn describe() -> Value {
    match current() {
        Ok(e) => serde_json::json!({ "name": e.name(), "capabilities": e.capabilities() }),
        Err(_) => super::none(),
    }
}
