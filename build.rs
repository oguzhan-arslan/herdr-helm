//! Puts the files that Helm installs into Herdr (mods, layouts, keys, templates) into the
//! program, so that `herdr-helm install` works without a clone of the repo. The list goes to
//! $OUT_DIR/assets.rs, which src/assets.rs includes.

use std::fs;
use std::path::{Path, PathBuf};

const DIRS: [&str; 4] = ["mods", "layouts", "keys", "templates"];

/// Files that Herdr or the OS writes into a mod folder: never part of Helm.
fn skip(name: &str) -> bool {
    name == ".DS_Store" || name == "gpui-kit.d.ts" || name == "jsconfig.json" || name.ends_with(".error")
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if skip(&e.file_name().to_string_lossy()) {
            continue;
        }
        if path.is_dir() {
            walk(&path, found);
        } else {
            found.push(path);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for d in DIRS {
        println!("cargo:rerun-if-changed={d}");
        walk(&root.join(d), &mut files);
    }
    files.sort();
    let mut out =
        String::from("/// Every file that Helm installs: (path in the repo, content).\npub static FILES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        // Cargo reruns this script when a file in a watched folder changes, but a new
        // subfolder needs its own line.
        if let Some(parent) = f.parent() {
            println!("cargo:rerun-if-changed={}", parent.display());
        }
        let rel = f.strip_prefix(&root).expect("file under the repo").to_string_lossy().replace('\\', "/");
        out.push_str(&format!("    ({rel:?}, include_bytes!({:?})),\n", f.display().to_string()));
    }
    out.push_str("];\n");
    let dest = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("assets.rs");
    fs::write(dest, out).expect("write assets.rs");
}
