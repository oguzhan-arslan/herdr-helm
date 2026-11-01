//! projects.json: the projects, their repos, the checks of a project, and the name rules.

use super::{record_path, truthy};
use crate::fail;
use crate::paths::{config_file, DEFAULT_TASKS_ROOT};
use crate::util::{expand, locked, read_json, write_json};
use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// The repos of a folder: (folder name, path), in the order of the names without case.
pub type Repos = Vec<(String, PathBuf)>;

/// The usual branch names: never a space name.
pub const RESERVED: [&str; 4] = ["main", "master", "develop", "head"];

/// A space name is also its folder, its branch and the start of its agents' hc names
/// (1-32 of a-z 0-9 - _), so it stays short and plain: 1-23 of a-z 0-9 -.
pub fn valid_space_name(name: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,22}$").expect("valid regex")).is_match(name)
}

/// The label of an extra checkout: 1-8 of a-z 0-9 -.
pub fn valid_label(label: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,7}$").expect("valid regex")).is_match(label)
}

/// projects.json as it is on disk, or {} when it is not there yet. A file that is not JSON
/// is an error: a save would lose what is in it.
pub fn read_raw() -> Result<Value> {
    let path = config_file();
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| anyhow!("{} is not JSON: {e}", path.display()))
}

/// The config, with "tasks_root" as a full path. No file yet: no projects.
pub fn config() -> Result<Value> {
    let mut c = read_raw()?;
    if !c.is_object() {
        fail!("{} is not a JSON object.", config_file().display());
    }
    let root = c["tasks_root"].as_str().unwrap_or(DEFAULT_TASKS_ROOT).to_string();
    c["tasks_root"] = json!(expand(&root).to_string_lossy());
    Ok(c)
}

/// The tasks folder of a config from `config()`.
pub fn tasks_root(c: &Value) -> PathBuf {
    expand(c["tasks_root"].as_str().unwrap_or(DEFAULT_TASKS_ROOT))
}

pub fn project<'a>(c: &'a Value, name: &str) -> Result<&'a Value> {
    match c.get("projects").and_then(|p| p.get(name)) {
        Some(p) => Ok(p),
        None => {
            let known: Vec<&str> = c["projects"].as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
            let known = if known.is_empty() { "none".to_string() } else { known.join(", ") };
            fail!("No project '{name}'. Known: {known}")
        }
    }
}

/// The git repos directly under a folder.
pub fn git_repos(root: &Path) -> Repos {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut found: Repos = entries
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.is_dir() && d.join(".git").exists())
        .filter_map(|d| Some((d.file_name()?.to_string_lossy().into_owned(), d)))
        .collect();
    found.sort_by_key(|(name, _)| name.to_lowercase());
    found
}

/// The root folder of a project.
pub fn project_root(p: &Value) -> PathBuf {
    expand(p["root"].as_str().unwrap_or(""))
}

/// The git repos directly under the project root, minus `exclude`.
pub fn project_repos(p: &Value) -> Repos {
    let skip = names(&p["exclude"]);
    git_repos(&project_root(p)).into_iter().filter(|(name, _)| !skip.contains(name)).collect()
}

/// The repo of that name in a list of repos.
pub fn find_repo<'a>(repos: &'a Repos, name: &str) -> Option<&'a PathBuf> {
    repos.iter().find(|(n, _)| n == name).map(|(_, p)| p)
}

/// The strings of a JSON list (other values are left out).
pub fn names(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Changes projects.json under its lock: reads the file that is on disk now, gives it to
/// `change`, and writes it back. Every key that `change` does not touch stays as it is.
pub fn update_config<T>(change: impl FnOnce(&mut Value) -> Result<T>) -> Result<T> {
    let path = config_file();
    locked(&path, || {
        let mut raw = if path.exists() { read_raw()? } else { json!({ "tasks_root": DEFAULT_TASKS_ROOT }) };
        if !raw.is_object() {
            fail!("{} is not a JSON object.", path.display());
        }
        let result = change(&mut raw)?;
        write_json(&path, &raw)?;
        Ok(result)
    })
}

/// A path that stays inside the folder it is joined to: relative, not empty, and with no ".."
/// part. Each `copy` and `link` entry of a repo must be one, because Helm copies, links and
/// removes these paths in a checkout.
pub fn inside_path(entry: &str) -> bool {
    let p = Path::new(entry);
    p.is_relative()
        && p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        && p.components().any(|c| matches!(c, Component::Normal(_)))
}

/// The settings of a repo in a project, and the JSON type of each.
const REPO_KEYS: [(&str, &str); 4] = [("copy", "list"), ("link", "list"), ("setup", "text"), ("base", "text")];

fn repo_key_ok(key: &str, value: &Value) -> bool {
    REPO_KEYS.iter().any(|(k, kind)| *k == key && if *kind == "list" { value.is_array() } else { value.is_string() })
}

fn is_name_list(v: Option<&Value>) -> bool {
    match v {
        None => true,
        Some(Value::Array(a)) => a.iter().all(Value::is_string),
        Some(_) => false,
    }
}

/// Refuses a project config that Helm could not use.
pub fn check_project(p: &Value) -> Result<()> {
    let root = project_root(p);
    if !root.is_dir() {
        fail!("The root folder {} does not exist.", root.display());
    }
    let found = git_repos(&root);
    if !p.get("base").is_none_or(Value::is_string) || !p.get("claude_md").is_none_or(Value::is_string) {
        fail!("'base' and 'claude_md' must be text.");
    }
    if let Some(t) = p["claude_md"].as_str().filter(|t| !t.is_empty()) {
        if !expand(t).is_file() {
            fail!("The CLAUDE.md template {t} does not exist.");
        }
    }
    for key in ["defaults", "exclude"] {
        if !is_name_list(p.get(key)) {
            fail!("'{key}' must be a list of names.");
        }
    }
    let (defaults, exclude) = (names(&p["defaults"]), names(&p["exclude"]));
    let unknown: Vec<&str> = defaults.iter().filter(|r| find_repo(&found, r).is_none()).map(String::as_str).collect();
    if !unknown.is_empty() {
        fail!("Defaults that are not repos under {}: {}", root.display(), unknown.join(", "));
    }
    let mut both: Vec<&str> = defaults.iter().filter(|r| exclude.contains(r)).map(String::as_str).collect();
    both.sort();
    both.dedup();
    if !both.is_empty() {
        fail!("A repo cannot be a default and excluded: {}", both.join(", "));
    }
    for (repo, settings) in p["repos"].as_object().into_iter().flatten() {
        let Some(settings) = settings.as_object() else { fail!("{repo}: the settings must be a JSON object.") };
        if let Some((key, _)) = settings.iter().find(|(key, value)| !repo_key_ok(key, value)) {
            fail!("{repo}: '{key}' is not a setting, or has the wrong type. Settings: copy, link (lists), setup, base (text).");
        }
        for key in ["copy", "link"] {
            if !is_name_list(settings.get(key)) {
                fail!("{repo}: '{key}' must be a list of paths.");
            }
            if let Some(bad) = names(&settings.get(key).cloned().unwrap_or_default()).into_iter().find(|f| !inside_path(f)) {
                fail!("{repo}: the {key} entry '{bad}' must be a path inside the checkout, with no '..'.");
            }
        }
    }
    Ok(())
}

/// Removes the settings of each repo that say nothing ("", [], null), then the repos with no
/// settings, then an empty "base" and "claude_md": it keeps the file short.
pub fn tidy_project(p: &mut Value) {
    if let Some(repos) = p.get_mut("repos").and_then(Value::as_object_mut) {
        for settings in repos.values_mut() {
            if let Some(s) = settings.as_object_mut() {
                s.retain(|_, v| !(v.is_null() || v == "" || v.as_array().is_some_and(Vec::is_empty)));
            }
        }
        repos.retain(|_, s| truthy(s));
    }
    if let Some(o) = p.as_object_mut() {
        for key in ["base", "claude_md"] {
            if !o.get(key).is_some_and(truthy) {
                o.remove(key);
            }
        }
    }
}

/// Inside a space, use the config that made it (a test config, or another Herdr home).
/// The program sets HERDR_PROJECTS once, at the start, so every later read of the config
/// and every child process agrees on it.
pub fn config_of_space() {
    if std::env::var_os("HERDR_PROJECTS").is_some() {
        return;
    }
    let Ok(cwd) = std::env::current_dir() else { return };
    let mut d = cwd.canonicalize().unwrap_or(cwd);
    while let Some(parent) = d.parent().map(Path::to_path_buf) {
        if record_path(&d).exists() {
            let rec = read_json(&record_path(&d), json!({}));
            if let Some(made_by) = rec["config"].as_str().filter(|m| Path::new(m).exists()) {
                std::env::set_var("HERDR_PROJECTS", made_by);
            }
            return;
        }
        d = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_rules() {
        assert!(valid_space_name("sso-login-times-out"));
        assert!(valid_space_name("a"));
        assert!(valid_space_name(&"a".repeat(23)));
        assert!(!valid_space_name(&"a".repeat(24)));
        assert!(!valid_space_name("-a"));
        assert!(!valid_space_name("Abc"));
        assert!(!valid_space_name("a_b"));
        assert!(!valid_space_name(""));
        assert!(valid_label("api"));
        assert!(valid_label("12345678"));
        assert!(!valid_label("123456789"));
        assert!(!valid_label("-x"));
    }

    fn root_with(repos: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for r in repos {
            std::fs::create_dir_all(dir.path().join(r).join(".git")).unwrap();
        }
        std::fs::create_dir_all(dir.path().join("not-a-repo")).unwrap();
        dir
    }

    #[test]
    fn repos_are_sorted_without_case() {
        let dir = root_with(&["web", "Api", "cli"]);
        let names: Vec<String> = git_repos(dir.path()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Api", "cli", "web"]);
        let p = json!({ "root": dir.path(), "exclude": ["cli"] });
        let names: Vec<String> = project_repos(&p).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Api", "web"]);
    }

    #[test]
    fn check_project_refuses_bad_configs() {
        let dir = root_with(&["web", "api"]);
        let root = dir.path().to_string_lossy().to_string();
        let ok = |p: Value| check_project(&p).map_err(|e| e.to_string());
        assert!(ok(json!({ "root": root, "defaults": ["web"], "exclude": ["api"], "repos": {} })).is_ok());
        assert!(ok(json!({ "root": root, "repos": { "web": { "copy": [".env"], "setup": "make" } } })).is_ok());
        let e = ok(json!({ "root": "/no/such/folder" })).unwrap_err();
        assert!(e.contains("does not exist"), "{e}");
        assert!(ok(json!({ "root": root, "base": 3 })).unwrap_err().contains("must be text"));
        assert!(ok(json!({ "root": root, "claude_md": "/no/such/file" })).unwrap_err().contains("template"));
        assert!(ok(json!({ "root": root, "defaults": "web" })).unwrap_err().contains("'defaults' must be a list"));
        assert!(ok(json!({ "root": root, "exclude": [1] })).unwrap_err().contains("'exclude' must be a list"));
        assert!(ok(json!({ "root": root, "defaults": ["nope"] })).unwrap_err().contains("nope"));
        let e = ok(json!({ "root": root, "defaults": ["web"], "exclude": ["web"] })).unwrap_err();
        assert!(e.contains("default and excluded: web"), "{e}");
        assert!(ok(json!({ "root": root, "repos": { "web": { "copy": ".env" } } })).unwrap_err().contains("web:"));
        assert!(ok(json!({ "root": root, "repos": { "web": { "colour": "x" } } })).unwrap_err().contains("web:"));
        assert!(ok(json!({ "root": root, "repos": { "web": [] } })).unwrap_err().contains("web:"));
        assert!(ok(json!({ "root": root, "repos": { "web": { "copy": [".env", "config/local.yml"], "link": ["node_modules"] } } })).is_ok());
        for bad in ["/etc/passwd", "../x", "a/../../b", "", "."] {
            for key in ["copy", "link"] {
                let e = ok(json!({ "root": root, "repos": { "web": { key: [bad] } } })).unwrap_err();
                assert!(e.contains("inside the checkout"), "{key} {bad}: {e}");
            }
        }
        assert!(ok(json!({ "root": root, "repos": { "web": { "copy": [1] } } })).unwrap_err().contains("list of paths"));
    }

    #[test]
    fn inside_paths() {
        assert!(inside_path(".env") && inside_path("a/b") && inside_path("./a") && inside_path("a/"));
        assert!(!inside_path("/a") && !inside_path("..") && !inside_path("a/../b") && !inside_path("") && !inside_path("./"));
    }

    #[test]
    fn tidy_drops_empty_settings() {
        let mut p = json!({ "root": "/x", "base": "", "claude_md": null,
            "repos": { "web": { "copy": [], "setup": "", "base": "dev" }, "api": { "link": [] } } });
        tidy_project(&mut p);
        assert_eq!(p, json!({ "root": "/x", "repos": { "web": { "base": "dev" } } }));
    }
}
