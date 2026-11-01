//! The core: config and projects, spaces and their checkouts, their agent files, and Herdr/hc.
//! docs/core.md describes the config, the space record, and each command.
//!
//!   config.rs       projects.json: the projects, their repos and checks, the name rules
//!   space.rs        the space record (.herdr/space.json) and the repos of a space or folder
//!   checkout.rs     the git worktrees of a space: add, state, changes, remove
//!   agent_files.rs  the CLAUDE.md and the skills of a space
//!   commands.rs     the commands of the core
//!   herdr.rs        Herdr and hc

mod agent_files;
mod checkout;
mod commands;
mod config;
pub mod herdr;
mod space;

pub use agent_files::{link_skills, write_agent_files};
pub use checkout::{add_checkout, checkout_status, losses, made_branch, remove_checkout_dir, repo_changes, repo_source};
// The code_host port builds its `github` rows on it.
#[allow(unused_imports)]
pub use checkout::repo_state;
pub use commands::*;
pub use config::{config, config_of_space, project, project_repos, tasks_root, valid_label, valid_space_name};
pub use space::{all_spaces, load_space, prs_of, record_path, save_space, space_repos, update_space};

use serde_json::Value;

/// True when a JSON value says something: null, false, 0, "", [] and {} are false.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A text field that is set and not empty.
pub fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_values_are_false() {
        for v in [json!(null), json!(false), json!(0), json!(""), json!([]), json!({})] {
            assert!(!truthy(&v), "{v}");
        }
        for v in [json!(true), json!(1), json!("a"), json!([1]), json!({"a": 1})] {
            assert!(truthy(&v), "{v}");
        }
    }
}

#[cfg(test)]
mod tests_space;
