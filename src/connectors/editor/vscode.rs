//! VS Code through its `code` command line. It finds `code` in this order: the setting
//! editor_settings.code, the PATH, the login shell's PATH (Herdr starts mods without it), and
//! the app bundle on macOS.

use crate::util::{exec, exec_with, expand, tool};
use anyhow::{anyhow, Result};
use std::path::Path;

pub struct VsCode {
    code: Option<String>,
}

impl VsCode {
    pub fn new() -> Self {
        let setting = crate::connectors::settings("editor")["code"].as_str().map(String::from);
        VsCode { code: find_code(setting.as_deref()) }
    }

    fn code(&self) -> Result<&str> {
        self.code
            .as_deref()
            .ok_or_else(|| anyhow!("VS Code's `code` command is not installed (VS Code: Shell Command: Install 'code' command in PATH)"))
    }

    fn run(&self, args: &[&str]) -> Result<()> {
        let out = exec(self.code()?, args, None, None)?;
        if !out.ok() {
            return Err(anyhow!("code {}: {}", args.join(" "), out.stderr.trim()));
        }
        Ok(())
    }
}

/// The `code` program. `setting` is editor_settings.code: a path ("~/" works), or a program
/// name on the PATH such as "code-insiders".
fn find_code(setting: Option<&str>) -> Option<String> {
    if let Some(found) = setting.map(str::trim).filter(|s| !s.is_empty()).and_then(from_setting) {
        return Some(found);
    }
    let on_path = tool("code");
    if Path::new(&on_path).is_file() {
        return Some(on_path);
    }
    for shell in ["/bin/zsh", "/bin/bash"] {
        // A slow login shell must not block the editor.
        if let Ok(out) = exec_with(shell, &["-lc", "command -v code"], None, None, Some(std::time::Duration::from_secs(10))) {
            let found = out.stdout.trim();
            if out.ok() && !found.is_empty() {
                return Some(found.to_string());
            }
        }
    }
    let bundle = "Visual Studio Code.app/Contents/Resources/app/bin/code";
    [Path::new("/Applications").join(bundle), crate::util::home().join("Applications").join(bundle)]
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

/// The program of editor_settings.code, when it is there.
fn from_setting(setting: &str) -> Option<String> {
    let path = if setting.contains('/') { expand(setting) } else { tool(setting).into() };
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

impl super::Editor for VsCode {
    fn name(&self) -> String {
        "vscode".into()
    }
    fn capabilities(&self) -> Vec<String> {
        if self.code.is_some() {
            vec!["open".into(), "diff".into(), "window".into()]
        } else {
            vec![]
        }
    }
    fn open(&self, path: &str) -> Result<()> {
        self.run(&[path])
    }
    fn diff(&self, old: &str, new: &str, reuse: bool) -> Result<()> {
        if reuse {
            self.run(&["-r", "--diff", old, new])
        } else {
            self.run(&["--diff", old, new])
        }
    }
    fn window(&self, folder: &str) -> Result<()> {
        self.run(&["-n", folder])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let code = dir.path().join("my-code");
        std::fs::write(&code, "").unwrap();
        let path = code.to_string_lossy().into_owned();
        assert_eq!(find_code(Some(&path)), Some(path.clone()));
        assert_eq!(from_setting(&path), Some(path));
        assert_eq!(from_setting(&dir.path().join("missing").to_string_lossy()), None);
    }
}
