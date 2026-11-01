//! The files that Helm installs into Herdr, inside the program (see build.rs). With them,
//! `herdr-helm install` and `herdr-helm update` need no clone of the repo.

use anyhow::Result;
use std::path::Path;

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// Writes the files into `dir`, in the same folders as in the repo.
pub fn extract(dir: &Path) -> Result<()> {
    for (rel, data) in FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, data)?;
    }
    Ok(())
}

/// The names of the Helm mods inside the program, such as "helm-bar".
pub fn mod_names() -> Vec<String> {
    let mut names: Vec<String> = FILES
        .iter()
        .filter_map(|(rel, _)| rel.strip_prefix("mods/")?.split_once('/').map(|(m, _)| m.to_string()))
        .filter(|m| m.starts_with("helm-"))
        .collect();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_has_what_an_install_needs() {
        let has = |p: &str| FILES.iter().any(|(rel, _)| *rel == p);
        assert!(has("layouts/helm.json"));
        assert!(has("keys/helm.json"));
        assert!(has("templates/CLAUDE.md"));
        assert!(has("mods/_shared/helm.js"));
        assert!(has("mods/helm-bar/main.js"));
        assert!(!FILES.iter().any(|(rel, _)| rel.ends_with(".DS_Store") || rel.ends_with(".error")));
        let mods = mod_names();
        assert!(mods.contains(&"helm-bar".to_string()) && !mods.contains(&"_shared".to_string()));
        assert!(mods.windows(2).all(|w| w[0] < w[1]), "{mods:?}");
    }

    #[test]
    fn extract_writes_every_file() {
        let dir = tempfile::tempdir().unwrap();
        extract(dir.path()).unwrap();
        for (rel, data) in FILES {
            assert_eq!(std::fs::read(dir.path().join(rel)).unwrap(), *data, "{rel}");
        }
    }
}
