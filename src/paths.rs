//! Where Helm keeps things. Everything derives from the Herdr home ($HERDR_APP_HOME, default
//! ~/.config/herdr-app), so a second Herdr home gets a second, separate Helm.
//!
//!   <home>/projects.json        the projects and the settings (HERDR_PROJECTS overrides it)
//!   <home>/helm/bin/herdr-helm  this program, as installed
//!   <home>/helm/templates/      the CLAUDE.md and the built-in skills of a space
//!   <home>/helm/state/          what Helm writes for itself: sessions, plain space titles,
//!                               the Smart rename work folder, background job files
//!   <home>/helm/install.json    what the last install did
//! The spaces themselves go to the tasks folder (projects.json "tasks_root", default
//! ~/herdr/tasks): they are the person's work, not Helm's state.

use crate::util::{expand, home};
use std::path::PathBuf;

pub fn app_home() -> PathBuf {
    match std::env::var("HERDR_APP_HOME") {
        Ok(p) if !p.is_empty() => expand(&p),
        _ => home().join(".config/herdr-app"),
    }
}

pub fn config_file() -> PathBuf {
    match std::env::var("HERDR_PROJECTS") {
        Ok(p) if !p.is_empty() => expand(&p),
        _ => app_home().join("projects.json"),
    }
}

pub fn helm_dir() -> PathBuf {
    app_home().join("helm")
}

pub fn state_dir() -> PathBuf {
    helm_dir().join("state")
}

/// The templates: next to the running program when it is installed (<helm>/bin/herdr-helm ->
/// <helm>/templates), else the repo's (a `cargo run` in a clone), else the installed ones.
pub fn templates_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HELM_TEMPLATES") {
        return expand(&p);
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.canonicalize().unwrap_or(exe);
        for up in exe.ancestors().skip(1).take(4) {
            let t = up.join("templates");
            if t.join("CLAUDE.md").is_file() {
                return t;
            }
        }
    }
    helm_dir().join("templates")
}

pub const DEFAULT_TASKS_ROOT: &str = "~/herdr/tasks";
