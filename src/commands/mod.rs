//! The commands that are not part of the core: each talks to one kind of connector.

pub mod agents; // the agent connector: agents, spawn, send, wait, sessions, restart, rename, info, usage
pub mod code_host; // the code_host connector: github, prs, pr
pub mod host; // the editor and system connectors, and the connectors overview: open, system, connectors
pub mod install; // install, uninstall
pub mod update; // update: newer releases of the source repo
