//! Shared helpers: running programs, git, the output of a command, and the error that every
//! command reports as "herdr-helm: <message>" with exit code 1.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// An error for the person: printed as "herdr-helm: <message>".
#[macro_export]
macro_rules! fail {
    ($($arg:tt)*) => { return Err(anyhow::anyhow!($($arg)*)) };
}

/// The output of a finished program.
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// The time limit of a call to a connector program or to `gh`.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs a program and waits for it, without a time limit. Never fails on a non-zero exit: look
/// at `code`.
pub fn exec(program: &str, args: &[&str], cwd: Option<&Path>, stdin: Option<&str>) -> Result<Output> {
    exec_with(program, args, cwd, stdin, None)
}

/// Runs a program and waits for it. A thread writes `stdin` while two others read stdout and
/// stderr, so a large input and a large output cannot block each other. With a `timeout`, the
/// program runs in its own process group, and the group is killed when the time runs out: that
/// is an error that names the program. Never fails on a non-zero exit: look at `code`.
pub fn exec_with(program: &str, args: &[&str], cwd: Option<&Path>, stdin: Option<&str>, timeout: Option<Duration>) -> Result<Output> {
    use std::io::{Read, Write};
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    if timeout.is_some() {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    // On Linux, a program that was just written can be "busy" for a moment: a fork in another
    // thread can hold the file open for writing until its exec. Try again for a short time.
    let mut tries = 0;
    let mut child = loop {
        match cmd.spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            other => break other.map_err(|e| anyhow!("{program}: {e}"))?,
        }
    };
    let writer = match (stdin, child.stdin.take()) {
        (Some(text), Some(mut pipe)) => {
            let text = text.to_string();
            // A program that exits without reading all of stdin makes the write fail: that is fine.
            Some(std::thread::spawn(move || drop(pipe.write_all(text.as_bytes()))))
        }
        _ => None,
    };
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Vec<u8>)>();
    let pipes: [Option<Box<dyn Read + Send>>; 2] = [
        child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
        child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
    ];
    for (i, pipe) in pipes.into_iter().enumerate() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut bytes);
            }
            let _ = tx.send((i, bytes));
        });
    }
    drop(tx);
    let deadline = timeout.map(|t| (Instant::now() + t, t));
    let timed_out = |child: &mut std::process::Child, limit: Duration| {
        kill_group(child);
        anyhow!("{program} did not finish in {limit:?} and was stopped")
    };
    let status = match deadline {
        None => child.wait()?,
        Some((end, limit)) => loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= end {
                return Err(timed_out(&mut child, limit));
            }
            std::thread::sleep(Duration::from_millis(10));
        },
    };
    // A child of the program can keep the pipes open after the program exits.
    let mut outputs = [Vec::new(), Vec::new()];
    for _ in 0..2 {
        let got = match deadline {
            None => rx.recv().ok(),
            Some((end, limit)) => match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
                Ok(got) => Some(got),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(timed_out(&mut child, limit)),
                Err(_) => None,
            },
        };
        if let Some((i, bytes)) = got {
            outputs[i] = bytes;
        }
    }
    if let Some(w) = writer {
        let _ = w.join();
    }
    let [stdout, stderr] = outputs;
    Ok(Output {
        code: status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// Kills the process group of a child that `exec_with` started with a timeout, then the child.
fn kill_group(child: &mut std::process::Child) {
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-KILL", "--", &group]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs a program and returns its trimmed stdout; fails with its stderr on a non-zero exit.
pub fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String> {
    run_with(program, args, cwd, None)
}

/// `run` with an optional time limit (see `exec_with`).
pub fn run_with(program: &str, args: &[&str], cwd: Option<&Path>, timeout: Option<Duration>) -> Result<String> {
    let out = exec_with(program, args, cwd, None, timeout)?;
    if !out.ok() {
        let shown: Vec<&str> = std::iter::once(program).chain(args.iter().copied()).take(4).collect();
        let why = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
        return Err(anyhow!("{} failed: {}", shown.join(" "), why));
    }
    Ok(out.stdout.trim().to_string())
}

/// `git -C <cwd> <args>`, trimmed. Fails on a non-zero exit.
pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut all = vec!["-C", cwd.to_str().unwrap_or(".")];
    all.extend_from_slice(args);
    run("git", &all, None)
}

/// `git -C <cwd> <args>` that may fail: None on a non-zero exit. Not trimmed when `raw`
/// (`status --porcelain` lines start with a space).
pub fn git_maybe(cwd: &Path, args: &[&str], raw: bool) -> Option<String> {
    let mut all = vec!["-C", cwd.to_str().unwrap_or(".")];
    all.extend_from_slice(args);
    let out = exec("git", &all, None, None).ok()?;
    if !out.ok() {
        return None;
    }
    Some(if raw { out.stdout } else { out.stdout.trim().to_string() })
}

/// Git options for reads that must not write under .git (an index lock wakes file watchers).
pub const READ: [&str; 5] = ["--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "diff.autoRefreshIndex=false"];

pub fn has_ref(repo: &Path, r#ref: &str) -> bool {
    git_maybe(repo, &["rev-parse", "--verify", "--quiet", r#ref], false).is_some_and(|s| !s.is_empty())
}

/// Prints the result of a command: JSON with --json, else the text for a person.
pub fn out(data: &Value, text: &str, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(data).unwrap_or_default());
    } else {
        println!("{text}");
    }
}

/// The user's home folder.
pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// "~/x" -> "<home>/x".
pub fn expand(path: &str) -> PathBuf {
    if path == "~" {
        return home();
    }
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

/// "<home>/x" -> "~/x", for messages.
pub fn short(path: &Path) -> String {
    let h = home();
    match path.strip_prefix(&h) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A program on the PATH, else in ~/.local/bin (where Herdr installs hc and herdr-app).
pub fn tool(name: &str) -> String {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = Path::new(dir).join(name);
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    home().join(".local/bin").join(name).to_string_lossy().into_owned()
}

/// Reads a JSON file; `default` when it is missing or not JSON.
pub fn read_json(path: &Path, default: Value) -> Value {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(default)
}

/// Writes a JSON file through a temporary file in the same folder, then renames it into place,
/// so a reader never sees half of it. Each write has its own temporary file, so two writers at
/// the same time do not use the same one. When `path` is a link, such as a projects.json that
/// a dotfiles repo links in, the write goes to the file it points to and the link stays.
pub fn write_json(path: &Path, data: &Value) -> Result<()> {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let target = if path.is_symlink() { std::fs::canonicalize(path).ok() } else { None };
    let path = target.as_deref().unwrap_or(path);
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
    let written = std::fs::write(&tmp, serde_json::to_string_pretty(data)? + "\n").and_then(|_| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(written?)
}

/// Runs `f` while this process holds the lock of `path`: the file `<path>.lock` next to it.
/// Use it around a read, change and write of a shared file (space.json, projects.json,
/// sessions.json), so that two writers do not lose each other's change. The lock is not the
/// data file itself, because `write_json` puts a new file in its place. Do not take the same
/// lock again inside `f`: the second lock waits for the first one forever.
pub fn locked<T>(path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lock = dir.join(format!("{name}.lock"));
    let file =
        std::fs::File::options().create(true).truncate(false).write(true).open(&lock).map_err(|e| anyhow!("{}: {e}", lock.display()))?;
    file.lock().map_err(|e| anyhow!("lock {}: {e}", lock.display()))?;
    // The lock goes when `file` closes, also when `f` fails.
    f()
}

/// Milliseconds since the epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Seconds since the epoch.
pub fn now_secs() -> i64 {
    now_ms() / 1000
}

/// Runs `f` on each item with at most `workers` threads, and keeps the order of the items.
/// Git reads of many repos in parallel take about as long as the slowest one.
pub fn par_map<T: Sync, R: Send>(items: &[T], workers: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..workers.min(items.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { break };
                let r = f(item);
                if let Ok(mut all) = results.lock() {
                    all[i] = Some(r);
                }
            });
        }
    });
    results.into_inner().unwrap_or_else(|e| e.into_inner()).into_iter().flatten().collect()
}

/// Tests that set environment variables (HERDR_PROJECTS and so on) hold this lock, because the
/// tests run in parallel threads of one process.
#[cfg(test)]
pub fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_json_keeps_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles/projects.json");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "{}").unwrap();
        let link = dir.path().join("projects.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_json(&link, &json!({ "a": 1 })).unwrap();
        assert!(link.is_symlink());
        assert_eq!(read_json(&real, Value::Null), json!({ "a": 1 }));
    }

    #[test]
    fn par_map_keeps_the_order() {
        let items: Vec<u32> = (0..50).collect();
        assert_eq!(par_map(&items, 8, |x| x * 2), items.iter().map(|x| x * 2).collect::<Vec<_>>());
        assert!(par_map(&Vec::<u32>::new(), 8, |x| *x).is_empty());
    }

    #[test]
    fn writers_at_the_same_time_do_not_share_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        let ok = par_map(&(0..8).collect::<Vec<u32>>(), 8, |i| (0..40).all(|n| write_json(&path, &json!({ "writer": i, "n": n })).is_ok()));
        assert!(ok.iter().all(|x| *x), "a write failed");
        assert!(read_json(&path, Value::Null).is_object());
        let left: Vec<String> =
            std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["data.json"]);
    }

    #[test]
    fn locked_changes_are_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("count.json");
        write_json(&path, &json!({ "n": 0 })).unwrap();
        par_map(&(0..4).collect::<Vec<u32>>(), 4, |_| {
            for _ in 0..25 {
                locked(&path, || {
                    let n = read_json(&path, json!({}))["n"].as_i64().unwrap_or(0);
                    write_json(&path, &json!({ "n": n + 1 }))
                })
                .unwrap();
            }
        });
        assert_eq!(read_json(&path, Value::Null)["n"], 100);
    }

    #[test]
    fn exec_passes_a_large_input_and_output_without_a_deadlock() {
        // More than a pipe buffer each way: cat writes while Helm still writes.
        let input = "x".repeat(1 << 20);
        let out = exec_with("cat", &[], None, Some(&input), Some(Duration::from_secs(20))).unwrap();
        assert_eq!((out.code, out.stdout.len()), (0, input.len()));
        let out = exec("sh", &["-c", "cat; echo err >&2; exit 3"], None, Some("hi")).unwrap();
        assert_eq!((out.code, out.stdout.as_str(), out.stderr.as_str()), (3, "hi", "err\n"));
    }

    #[test]
    fn exec_stops_a_program_at_its_timeout() {
        let start = Instant::now();
        let e = exec_with("sh", &["-c", "sleep 30; echo late"], None, None, Some(Duration::from_millis(300))).err().expect("timeout");
        assert!(e.to_string().starts_with("sh did not finish in 300ms"), "{e}");
        assert!(start.elapsed() < Duration::from_secs(5));
        // A child that keeps the pipes open after the program exits.
        let e = exec_with("sh", &["-c", "sleep 30 &"], None, None, Some(Duration::from_millis(300))).err().expect("timeout");
        assert!(e.to_string().contains("did not finish"), "{e}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
