//! Checks of the mods that need no Herdr: the mod.json files, the overlay sizes and names, the
//! shared files, the icon paths, the Herdr mods that Helm uses, the key commands, and names that
//! must not come back.

use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The mod folders (helm-*), without mods/_shared.
fn mods() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(repo().join("mods"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().unwrap().to_string_lossy().starts_with("helm-"))
        .collect();
    found.sort();
    found
}

/// Every .js file of the mods, with mods/_shared.
fn scripts() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in mods().into_iter().chain(std::iter::once(repo().join("mods/_shared"))) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            if e.path().extension().is_some_and(|x| x == "js") {
                found.push(e.path());
            }
        }
    }
    found
}

/// OVERLAYS of mods/_shared/helm.js: name -> (mod, kind, width, height).
fn overlays() -> Vec<(String, String, String, u64, u64)> {
    let text = fs::read_to_string(repo().join("mods/_shared/helm.js")).unwrap();
    let line = Regex::new(r#"(?m)^\s*"([\w-]+)": \{ (?:mod: "([\w-]+)", )?kind: "(\w+)", width: (\d+), height: (\d+) \},$"#).unwrap();
    line.captures_iter(&text)
        .map(|c| {
            let name = c[1].to_string();
            let module = c.get(2).map_or(name.clone(), |m| m.as_str().to_string());
            (name, module, c[3].to_string(), c[4].parse().unwrap(), c[5].parse().unwrap())
        })
        .collect()
}

/// HERDR_MODS of src/commands/install.rs: the Herdr mods that the installer looks for.
fn herdr_mods() -> Vec<String> {
    let text = fs::read_to_string(repo().join("src/commands/install.rs")).unwrap();
    let list = Regex::new(r"const HERDR_MODS: \[&str; \d+\] = \[([^\]]*)\];").unwrap();
    let names = list.captures(&text).expect("cannot read HERDR_MODS in src/commands/install.rs");
    Regex::new(r#""([\w-]+)""#).unwrap().captures_iter(&names[1]).map(|c| c[1].to_string()).collect()
}

/// The text of each script, with its path.
fn texts() -> Vec<(PathBuf, String)> {
    scripts().into_iter().map(|p| (p.clone(), fs::read_to_string(p).unwrap())).collect()
}

/// The names that an icon template can make: `mods/helm-x/chevron-${open ? "down" : "right"}.svg`
/// gives chevron-down and chevron-right. A template with no text in its `${}` gives nothing,
/// because its names are not known here.
fn template_names(name: &str) -> Vec<String> {
    let Some(start) = name.find("${") else { return vec![name.to_string()] };
    let Some(len) = name[start..].find('}') else { return Vec::new() };
    let (head, expr, tail) = (&name[..start], &name[start + 2..start + len], &name[start + len + 1..]);
    let words: Vec<String> = Regex::new(r#""([\w-]*)""#).unwrap().captures_iter(expr).map(|c| c[1].to_string()).collect();
    words.iter().flat_map(|w| template_names(&format!("{head}{w}{tail}"))).collect()
}

#[test]
fn every_mod_json_has_the_same_form() {
    for dir in mods() {
        let path = dir.join("mod.json");
        let text = fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} has no mod.json", dir.display()));
        let data: Value = serde_json::from_str(&text).unwrap();
        // Without "place": "none", Herdr puts the mod into the shown layout.
        assert_eq!(data["place"], "none", "{}", path.display());
        assert!(data["title"].as_str().is_some_and(|t| !t.is_empty()), "{} has no title", path.display());
        assert!(text.ends_with("}\n"), "{} must end with one newline", path.display());
        assert_eq!(text, serde_json::to_string_pretty(&data).unwrap() + "\n", "{} is not in the usual form", path.display());
    }
}

#[test]
fn mod_json_overlays_match_the_overlay_table() {
    let table = overlays();
    assert!(table.len() >= 10, "cannot read OVERLAYS in mods/_shared/helm.js");
    for dir in mods() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let data: Value = serde_json::from_str(&fs::read_to_string(dir.join("mod.json")).unwrap()).unwrap();
        let Some(o) = data.get("overlay") else { continue };
        let row = table.iter().find(|r| r.0 == name).unwrap_or_else(|| panic!("{name}: an overlay in mod.json, but not in OVERLAYS"));
        assert_eq!(row.1, name);
        assert_eq!(
            (o["kind"].as_str().unwrap(), o["width"].as_u64().unwrap(), o["height"].as_u64().unwrap()),
            (row.2.as_str(), row.3, row.4),
            "{name}"
        );
    }
}

#[test]
fn shared_imports_resolve() {
    let import = Regex::new(r#"from "\./([\w.-]+)""#).unwrap();
    for path in scripts() {
        let text = fs::read_to_string(&path).unwrap();
        for c in import.captures_iter(&text) {
            let file = &c[1];
            let here = path.parent().unwrap().join(file);
            let shared = repo().join("mods/_shared").join(file);
            assert!(here.is_file() || shared.is_file(), "{}: no ./{file}", path.display());
        }
    }
}

#[test]
fn helm_icons_exist() {
    let icon = Regex::new(r#"["`](mods/(helm-[\w-]+)/([\w-]+)\.svg)["`]"#).unwrap();
    for (path, text) in texts() {
        for c in icon.captures_iter(&text) {
            assert!(repo().join(&c[1]).is_file(), "{}: no {}", path.display(), &c[1]);
        }
    }
}

#[test]
fn icon_templates_point_to_existing_files() {
    // A template string: the folder must exist, and each name it can make must be a file.
    let template = Regex::new(r#"`mods/(helm-[\w-]+)/([^`]*)\.svg`"#).unwrap();
    let mut checked = 0;
    for (path, text) in texts() {
        for c in template.captures_iter(&text) {
            let dir = repo().join("mods").join(&c[1]);
            assert!(dir.is_dir(), "{}: no folder mods/{}", path.display(), &c[1]);
            for name in template_names(&c[2]) {
                assert!(dir.join(format!("{name}.svg")).is_file(), "{}: no mods/{}/{name}.svg", path.display(), &c[1]);
                checked += 1;
            }
        }
    }
    assert!(checked >= 2, "found no icon templates: the pattern is out of date");
}

#[test]
fn herdr_mods_that_helm_uses_are_installed() {
    // A path into a Herdr mod (not helm-*) needs that mod, and the installer warns when it is
    // missing only when HERDR_MODS lists it.
    let known = herdr_mods();
    assert!(known.iter().any(|m| m == "workspace-chrome"), "cannot read HERDR_MODS");
    let used = Regex::new(r#"["`/]mods/([\w-]+)/"#).unwrap();
    for (path, text) in texts() {
        for c in used.captures_iter(&text) {
            let name = &c[1];
            if name.starts_with("helm-") {
                assert!(repo().join("mods").join(name).is_dir(), "{}: no mod {name}", path.display());
            } else {
                assert!(
                    known.iter().any(|m| m == name),
                    "{}: uses mods/{name}/, which HERDR_MODS in src/commands/install.rs does not list",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn overlays_that_mods_open_exist() {
    let names: Vec<String> = overlays().into_iter().map(|r| r.0).collect();
    // The first argument of overlay(...), and each name in it: "a", or the results of a
    // ternary (after its "?").
    let call = Regex::new(r"\boverlay\((?:[^,)?]*\?)?([^,)]*)").unwrap();
    let index = Regex::new(r#"OVERLAYS\["([\w-]+)"\]"#).unwrap();
    let word = Regex::new(r#""([\w-]+)""#).unwrap();
    let mut checked = 0;
    for (path, text) in texts() {
        let calls = call
            .captures_iter(&text)
            .flat_map(|c| word.captures_iter(c.get(1).unwrap().as_str()).map(|w| w[1].to_string()).collect::<Vec<_>>());
        for name in calls.chain(index.captures_iter(&text).map(|c| c[1].to_string())) {
            assert!(names.contains(&name), "{}: overlay {name:?} is not in OVERLAYS", path.display());
            checked += 1;
        }
    }
    assert!(checked >= 10, "found few overlay calls: the pattern is out of date");
}

#[test]
fn old_names_do_not_come_back() {
    let old = Regex::new(r#"herdr-task|\.local/bin|claude-sessions\.json|space-titles\.json|"focus\.|Focus(Menu|Github|bar|menu)|icons/claude\.svg|\.config/herdr-app"#).unwrap();
    let mut files = scripts();
    files.extend(mods().iter().map(|d| d.join("mod.json")));
    files.extend(["keys/helm.json", "layouts/helm.json"].iter().map(|f| repo().join(f)));
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        if let Some(m) = old.find(&text) {
            panic!("{}: old name {:?}", path.display(), m.as_str());
        }
    }
}

#[test]
fn old_names_are_not_in_the_docs_or_templates() {
    // The names of the older Python version. A doc may name them only in a section about an
    // older install, which the heading says.
    let old = Regex::new(
        r#"herdr-task|claude-sessions\.json|~/herdr/space-titles\.json|\.local/bin/herdr-task|"focus\.|Focus(Menu|Github|bar|menu)"#,
    )
    .unwrap();
    let history = Regex::new(r"(?i)^#+ .*\b(older|history|migration)\b").unwrap();
    let mut files = Vec::new();
    for dir in ["docs", "templates"] {
        let mut stack = vec![repo().join(dir)];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(d).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    files.push(e.path());
                }
            }
        }
    }
    files.extend(["README.md", "AGENTS.md"].iter().map(|f| repo().join(f)));
    assert!(files.len() > 5, "found no docs");
    for path in files {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let mut past = false;
        for (n, line) in text.lines().enumerate() {
            if line.starts_with('#') {
                past = history.is_match(line);
            }
            if let Some(m) = old.find(line).filter(|_| !past) {
                panic!("{}:{}: old name {:?}", path.display(), n + 1, m.as_str());
            }
        }
    }
}

#[test]
fn keys_name_commands_of_existing_mods() {
    let keys: Value = serde_json::from_str(&fs::read_to_string(repo().join("keys/helm.json")).unwrap()).unwrap();
    for (section, map) in keys.as_object().unwrap() {
        for (key, action) in map.as_object().unwrap() {
            let Some(command) = action.as_str() else { continue };
            if let Some((module, _)) = command.split_once('.').filter(|(m, _)| m.starts_with("helm-")) {
                assert!(Path::new(&repo().join("mods").join(module)).is_dir(), "{section} {key}: no mod {module}");
            }
        }
    }
}

#[test]
fn mods_handle_their_key_commands() {
    // Each "<mod>.<command>" of keys/helm.json and of the menu must reach code in that mod's
    // main.js: c === "<command>", or a regex such as /^tile[1-9]$/.test(c).
    let keys: Value = serde_json::from_str(&fs::read_to_string(repo().join("keys/helm.json")).unwrap()).unwrap();
    let mut commands: Vec<String> = keys
        .as_object()
        .unwrap()
        .values()
        .flat_map(|map| map.as_object().unwrap().values().filter_map(|a| a.as_str().map(String::from)).collect::<Vec<_>>())
        .collect();
    let named = Regex::new(r#""(helm-[\w-]+\.\w+)""#).unwrap();
    for (_, text) in texts() {
        commands.extend(named.captures_iter(&text).map(|c| c[1].to_string()));
    }
    let pattern = Regex::new(r"/(\^[^/]+\$)/\.test\(c\)").unwrap();
    let mut checked = 0;
    for command in commands {
        let Some((module, name)) = command.split_once('.').filter(|(m, _)| m.starts_with("helm-")) else { continue };
        let Ok(text) = fs::read_to_string(repo().join("mods").join(module).join("main.js")) else { continue };
        let exact = text.contains(&format!("c === \"{name}\""));
        let matched = pattern.captures_iter(&text).any(|c| Regex::new(&c[1]).is_ok_and(|r| r.is_match(name)));
        assert!(exact || matched, "{command}: mods/{module}/main.js does not handle {name:?}");
        checked += 1;
    }
    assert!(checked >= 20, "found few key commands");
}

#[test]
fn shared_values_are_in_the_architecture_doc() {
    // Each key of shared("x") and setShared("x") must be named in docs/architecture.md. A key
    // made from a template (`helm.project.${name}`) is named as `helm.project.<name>`.
    let doc = fs::read_to_string(repo().join("docs/architecture.md")).unwrap();
    let key = Regex::new(r#"(?:setShared|shared)\((?:"([\w.-]+)"|`([\w.-]+)\$\{)"#).unwrap();
    let mut checked = 0;
    for (path, text) in texts() {
        for c in key.captures_iter(&text) {
            let name = c.get(1).map_or_else(|| format!("{}<", &c[2]), |m| format!("{}`", m.as_str()));
            assert!(doc.contains(&format!("`{name}")), "{}: shared value {name:?} is not in docs/architecture.md", path.display());
            checked += 1;
        }
    }
    assert!(checked >= 20, "found few shared values");
}
