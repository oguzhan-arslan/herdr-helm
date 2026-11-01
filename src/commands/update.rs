//! update: Helm checks the GitHub releases of its source repo for a newer version, and installs
//! one only when the person asks. `update --check` is cheap: it asks GitHub at most every
//! CHECK_HOURS and keeps the answer in state/update.json. helm-jobs runs it, and the ⋯ menu shows
//! the update. `update` downloads the release program for this machine, checks its SHA-256 sum,
//! and runs its `install`, which installs the files inside that program.
//!
//! The source repo is `repository` of Cargo.toml. HELM_REPO=<owner>/<name> sets another one, for
//! a fork. A private repo needs `gh`, signed in. A public repo also works with curl.

use crate::cli::*;
use crate::util::{exec_with, now_ms, out, read_json, write_json, Output, CALL_TIMEOUT};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

const CHECK_HOURS: i64 = 6;
const HOST: &str = "github.com";

/// owner/name of the source repo.
pub fn slug() -> String {
    if let Ok(s) = std::env::var("HELM_REPO") {
        return s.trim().trim_matches('/').to_string();
    }
    env!("CARGO_PKG_REPOSITORY")
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplitn(3, '/')
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("/")
}

/// (major, minor, patch) of "v1.2.3" or "1.2.3". A suffix such as "-rc.1" is ignored.
pub fn version_of(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

/// Whether release `latest` is newer than the running program.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((version_of(latest), version_of(current)), (Some(l), Some(c)) if l > c)
}

/// The release file name of this machine, as the CI names it.
pub fn asset_name() -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => return Err(anyhow!("no release build for the CPU type {other}")),
    };
    let os = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-musl",
        other => return Err(anyhow!("no release build for {other}")),
    };
    Ok(format!("herdr-helm-{arch}-{os}.tar.gz"))
}

/// The time limit of a release download, and of the install step of the new program.
const LONG_CALL: Duration = Duration::from_secs(300);

/// A call with a time limit, so that a stuck network call never blocks the mods.
fn exec(program: &str, args: &[&str], limit: Duration) -> Result<Output> {
    exec_with(program, args, None, None, Some(limit))
}

fn have(program: &str) -> bool {
    exec(program, &["--version"], CALL_TIMEOUT).is_ok_and(|o| o.ok())
}

/// The newest release: {"tag", "url", "notes"}. Uses gh when it can (private repos). When gh is
/// missing or fails, for example when it is not signed in, it uses curl.
fn latest_release() -> Result<Value> {
    let path = format!("repos/{}/releases/latest", slug());
    let by_gh = have("gh").then(|| exec("gh", &["api", "--hostname", HOST, &path], CALL_TIMEOUT)).and_then(Result::ok).filter(Output::ok);
    let text = match by_gh {
        Some(o) => o.stdout,
        None => {
            let url = format!("https://api.{HOST}/{path}");
            let o = exec("curl", &["-fsSL", "--max-time", "20", "-H", "Accept: application/vnd.github+json", &url], CALL_TIMEOUT)?;
            if !o.ok() {
                return Err(anyhow!("cannot read {url}. For a private repo, install gh and run 'gh auth login'."));
            }
            o.stdout
        }
    };
    let r: Value = serde_json::from_str(&text).map_err(|e| anyhow!("the release reply is not JSON: {e}"))?;
    let tag = r["tag_name"].as_str().ok_or_else(|| anyhow!("the release has no tag"))?;
    Ok(json!({ "tag": tag, "url": r["html_url"], "notes": r["body"] }))
}

/// The answer of `update --check`, from a reply or an error.
fn check_result(release: &Result<Value>, current: &str, at: i64) -> Value {
    match release {
        Ok(r) => {
            let tag = r["tag"].as_str().unwrap_or("");
            json!({ "current": current, "latest": tag, "available": is_newer(tag, current), "url": r["url"], "checked_at": at, "error": null })
        }
        Err(e) => {
            json!({ "current": current, "latest": null, "available": false, "url": null, "checked_at": at, "error": format!("{e:#}") })
        }
    }
}

fn cache_file() -> std::path::PathBuf {
    crate::paths::state_dir().join("update.json")
}

/// The cached answer when it is fresh and for this version, else a new one.
fn check(force: bool) -> Result<Value> {
    let current = env!("CARGO_PKG_VERSION");
    let cached = read_json(&cache_file(), Value::Null);
    let fresh = cached["checked_at"].as_i64().is_some_and(|t| now_ms() - t < CHECK_HOURS * 3_600_000);
    if !force && fresh && cached["current"] == current {
        return Ok(cached);
    }
    let result = check_result(&latest_release(), current, now_ms());
    write_json(&cache_file(), &result)?;
    Ok(result)
}

fn sha256(file: &Path) -> Result<String> {
    let f = file.to_string_lossy();
    let o = if have("sha256sum") { exec("sha256sum", &[&f], CALL_TIMEOUT)? } else { exec("shasum", &["-a", "256", &f], CALL_TIMEOUT)? };
    o.stdout.split_whitespace().next().map(String::from).filter(|_| o.ok()).ok_or_else(|| anyhow!("cannot compute the SHA-256 sum"))
}

/// Downloads `name` and `name.sha256` of release `tag` into `dir`.
fn download(tag: &str, name: &str, dir: &Path) -> Result<()> {
    let d = dir.to_string_lossy();
    let repo = format!("{HOST}/{}", slug());
    let sum = format!("{name}.sha256");
    let by_gh = have("gh")
        && exec("gh", &["release", "download", tag, "-R", &repo, "-p", name, "-p", &sum, "-D", &d], LONG_CALL).is_ok_and(|o| o.ok());
    if by_gh {
        return Ok(());
    }
    let base = format!("https://{HOST}/{}/releases/download/{tag}", slug());
    for f in [name, sum.as_str()] {
        let target = dir.join(f);
        let o = exec("curl", &["-fsSL", "--max-time", "280", "-o", &target.to_string_lossy(), &format!("{base}/{f}")], LONG_CALL)?;
        if !o.ok() {
            return Err(anyhow!("cannot download {f} of {tag}. For a private repo, install gh and run 'gh auth login'."));
        }
    }
    Ok(())
}

/// Downloads, checks, and installs the newest release.
fn apply(c: &Ctx) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let release = latest_release()?;
    let tag = release["tag"].as_str().unwrap_or_default().to_string();
    if !is_newer(&tag, current) {
        let data = json!({ "updated": false, "current": current, "latest": tag });
        out(&data, &format!("Helm {current} is the newest version."), c.json);
        return Ok(());
    }
    let name = asset_name()?;
    let tmp = TempDir::new()?;
    let tmp = tmp.0.as_path();
    download(&tag, &name, tmp)?;
    let want = std::fs::read_to_string(tmp.join(format!("{name}.sha256")))?.split_whitespace().next().unwrap_or_default().to_string();
    if sha256(&tmp.join(&name))? != want {
        return Err(anyhow!("the SHA-256 sum of {name} is wrong. Nothing changed."));
    }
    let o = exec("tar", &["-xzf", &tmp.join(&name).to_string_lossy(), "-C", &tmp.to_string_lossy()], CALL_TIMEOUT)?;
    if !o.ok() {
        return Err(anyhow!("cannot unpack {name}: {}", o.stderr.trim()));
    }
    // The new program installs itself and the files inside it.
    let new = tmp.join("herdr-helm");
    let o = exec(&new.to_string_lossy(), &["install"], LONG_CALL)?;
    if !o.ok() {
        return Err(anyhow!("the install of {tag} failed: {}", o.stderr.trim().lines().last().unwrap_or("")));
    }
    let _ = std::fs::remove_file(cache_file());
    let data = json!({ "updated": true, "from": current, "to": tag });
    out(&data, &format!("Helm is now {tag} (was {current})."), c.json);
    Ok(())
}

/// A new, empty folder in the state folder (the same disk as the install, so a rename works).
/// It goes when the update ends, also when the update fails.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Result<TempDir> {
        let dir = crate::paths::state_dir().join("update").join(format!("{:x}-{}", now_ms(), std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn cmd_update(c: &Ctx, a: &UpdateArgs) -> Result<()> {
    if !a.check {
        return apply(c);
    }
    let r = check(a.force)?;
    let text = match (&r["available"], r["latest"].as_str(), r["error"].as_str()) {
        (Value::Bool(true), Some(l), _) => {
            format!("Helm {l} is available (you have {}). Run 'herdr-helm update'.", r["current"].as_str().unwrap_or(""))
        }
        (_, _, Some(e)) => format!("Cannot check for updates: {e}"),
        _ => format!("Helm {} is the newest version.", r["current"].as_str().unwrap_or("")),
    };
    out(&r, &text, c.json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        assert!(!is_newer("v0.2.0-rc.1", "0.2.0"));
        assert!(!is_newer("nonsense", "0.1.0"));
    }

    #[test]
    fn the_source_repo_comes_from_cargo_toml() {
        let _env = crate::util::env_lock();
        std::env::remove_var("HELM_REPO");
        assert_eq!(slug(), "oguzhan-arslan/herdr-helm");
        std::env::set_var("HELM_REPO", "someone/fork/");
        assert_eq!(slug(), "someone/fork");
        std::env::remove_var("HELM_REPO");
    }

    #[test]
    fn a_failed_check_is_no_update() {
        let r = check_result(&Err(anyhow!("offline")), "0.1.0", 5);
        assert_eq!(r["available"], false);
        assert_eq!(r["error"], "offline");
        let r = check_result(&Ok(json!({ "tag": "v0.2.0", "url": "u" })), "0.1.0", 5);
        assert_eq!(r["available"], true);
        assert_eq!(r["latest"], "v0.2.0");
    }

    #[test]
    fn the_asset_name_matches_the_ci() {
        let name = asset_name().unwrap();
        assert!(name.starts_with("herdr-helm-") && name.ends_with(".tar.gz"), "{name}");
    }
}
