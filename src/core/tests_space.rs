//! Whole spaces, from project add to delete, in a temporary folder with real git repos. The
//! tests here set HERDR_PROJECTS, so each one holds `env_lock`. They call `complete` and
//! `delete` without the terminal check, because a developer may run the tests in a terminal.

use super::*;
use crate::cli::*;
use serde_json::json;
use std::path::Path;
use std::process::Command;

fn sh(dir: &Path, cmd: &str) {
    let ok = Command::new("sh").args(["-c", cmd]).current_dir(dir).output().unwrap();
    assert!(ok.status.success(), "{cmd}: {}", String::from_utf8_lossy(&ok.stderr));
}

fn make_repo(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    sh(&dir, "git init -q -b main && git -c user.name=t -c user.email=t@t commit -q --allow-empty -m first && echo secret > .env");
}

fn ctx(space: Option<&str>) -> Ctx {
    Ctx { json: true, space: space.map(String::from) }
}

#[test]
fn space_lifecycle() {
    let _env = crate::util::env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let (root, tasks) = (tmp.path().join("repos"), tmp.path().join("tasks"));
    make_repo(&root, "web");
    make_repo(&root, "api");
    let cfg = tmp.path().join("projects.json");
    std::fs::write(&cfg, json!({ "tasks_root": tasks }).to_string()).unwrap();
    std::env::set_var("HERDR_PROJECTS", &cfg);
    std::env::set_var("HELM_TEMPLATES", Path::new(env!("CARGO_MANIFEST_DIR")).join("templates"));

    let add = ProjectArgs {
        action: "add".into(),
        name: "demo".into(),
        root: Some(root.to_string_lossy().into()),
        default: vec!["web".into()],
        data: None,
    };
    cmd_project(&ctx(None), &add).unwrap();
    let data = r#"{"repos": {"web": {"copy": [".env"], "setup": "echo $HERDR_SPACE > ../setup-ran", "link": []}}}"#;
    let update = ProjectArgs { action: "update".into(), name: "demo".into(), root: None, default: vec![], data: Some(data.into()) };
    cmd_project(&ctx(None), &update).unwrap();
    let c = config().unwrap();
    assert_eq!(project(&c, "demo").unwrap()["repos"], json!({ "web": { "copy": [".env"], "setup": "echo $HERDR_SPACE > ../setup-ran" } }));

    let new = NewArgs {
        name: None,
        title: Some("Fix the login".into()),
        project: "demo".into(),
        repo: vec![],
        goal: None,
        open: false,
        agent: "agent".into(),
        progress: false,
    };
    cmd_new(&ctx(None), &new).unwrap();
    let (rec, dir) = load_space(&c, Some("fix-the-login")).unwrap();
    assert_eq!(rec["goal"], "Fix the login");
    assert_eq!(rec["checkouts"][0]["base"], "main");
    assert!(dir.join("web/.env").is_file());
    assert_eq!(std::fs::read_to_string(dir.join("setup-ran")).unwrap().trim(), "fix-the-login");
    let claude_md = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
    assert!(claude_md.contains("| `web/` | web | `fix-the-login` | main |") && claude_md.contains("Repos you can add: `api`"));
    assert!(dir.join(".claude/skills/task-workspace/SKILL.md").is_file());
    assert!(dir.join("web/.claude/skills/task-workspace").is_symlink());
    // The copied .env is local to the checkout, the skill link is excluded: the checkout is clean.
    assert_eq!(checkout_status(&rec, &rec["checkouts"][0]).unwrap()["dirty"], 0);

    // The same title again gets the next free name.
    cmd_new(&ctx(None), &NewArgs { repo: vec!["api".into()], ..new }).unwrap();
    assert!(tasks.join("fix-the-login-2/api").is_dir());

    cmd_add(&ctx(Some("fix-the-login")), &AddArgs { repo: "web".into(), label: Some("review".into()) }).unwrap();
    let (rec, _) = load_space(&c, Some("fix-the-login")).unwrap();
    assert_eq!(rec["checkouts"][1]["branch"], "fix-the-login-review");
    assert_eq!(rec["checkouts"][1]["base"], "fix-the-login");
    assert_eq!(checkout_status(&rec, &rec["checkouts"][1]).unwrap()["merged"], true);

    std::fs::write(dir.join("web/notes.txt"), "a\nb\n").unwrap();
    let (_, repos) = space_repos(Some("fix-the-login"), None, None).unwrap();
    let changes = repo_changes(&repos[0].0, &repos[0].1);
    // `changes` lists the copied .env too: only `status` leaves it out.
    assert_eq!(changes["files"][1], json!({ "path": "notes.txt", "status": "A", "add": 2, "del": 0, "untracked": true }));
    // The repos have no remote, so their first commit is not pushed either.
    let lost = losses(&rec, &dir).unwrap();
    assert_eq!(lost[0], json!({ "checkout": "web", "what": "1 uncommitted changes" }));
    assert_eq!(lost[1], json!({ "checkout": "web", "what": "1 commits on fix-the-login not pushed" }));
    // The setup wrote a file into the space folder, outside every checkout.
    assert_eq!(lost.last().unwrap(), &json!({ "checkout": "", "what": "setup-ran is not in a checkout" }));

    cmd_remove(&ctx(Some("fix-the-login")), &RemoveArgs { checkout: "web.review".into() }).unwrap();
    assert!(!dir.join("web.review").exists());
    assert!(!crate::util::has_ref(&root.join("web"), "refs/heads/fix-the-login-review"));

    // Refresh writes back a CLAUDE.md that was changed by hand.
    std::fs::write(dir.join("CLAUDE.md"), "old").unwrap();
    cmd_refresh(&ctx(None)).unwrap();
    assert!(std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap().contains("# Space: fix-the-login"));

    for name in ["fix-the-login", "fix-the-login-2"] {
        super::commands::delete_space(&ctx(Some(name)), &DeleteArgs { yes: true, close: false }).unwrap();
        assert!(!tasks.join(name).exists());
    }
    assert!(!crate::util::has_ref(&root.join("web"), "refs/heads/fix-the-login"));
    assert!(all_spaces(&c).unwrap().is_empty());
}

/// A temporary Helm: repos with a bare origin each, a config, and the env vars that point to
/// them. Hold `env_lock` while it lives.
struct Helm {
    tmp: tempfile::TempDir,
}

impl Helm {
    fn new(repos: &[&str], settings: serde_json::Value) -> Helm {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repos");
        for r in repos {
            make_repo(&root, r);
            let bare = tmp.path().join("origins").join(r);
            std::fs::create_dir_all(&bare).unwrap();
            sh(&bare, "git init -q --bare");
            sh(
                &root.join(r),
                &format!("git remote add origin {} && git push -q origin main 2>/dev/null && git fetch -q origin", bare.display()),
            );
        }
        let cfg = tmp.path().join("projects.json");
        let project = json!({ "root": root, "defaults": [repos[0]], "exclude": [], "repos": settings });
        std::fs::write(&cfg, json!({ "tasks_root": tmp.path().join("tasks"), "projects": { "demo": project } }).to_string()).unwrap();
        std::env::set_var("HERDR_PROJECTS", &cfg);
        std::env::set_var("HELM_TEMPLATES", Path::new(env!("CARGO_MANIFEST_DIR")).join("templates"));
        Helm { tmp }
    }

    fn path(&self, rel: &str) -> std::path::PathBuf {
        self.tmp.path().join(rel)
    }

    fn new_space(&self, name: &str) -> anyhow::Result<()> {
        let new = NewArgs {
            name: Some(name.into()),
            title: None,
            project: "demo".into(),
            repo: vec![],
            goal: None,
            open: false,
            agent: "shell".into(),
            progress: false,
        };
        cmd_new(&ctx(None), &new)
    }
}

#[test]
fn new_removes_the_worktree_and_branch_when_a_copy_fails() {
    let _env = crate::util::env_lock();
    let helm = Helm::new(&["web"], json!({ "web": { "copy": [".env"] } }));
    let env_file = helm.path("repos/web/.env");
    std::fs::set_permissions(&env_file, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let made = helm.new_space("broken");
    std::fs::set_permissions(&env_file, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    assert!(made.is_err());
    let web = helm.path("repos/web");
    assert!(!crate::util::has_ref(&web, "refs/heads/broken"));
    assert!(!crate::util::git(&web, &["worktree", "list"]).unwrap().contains("broken"));
    assert!(!helm.path("tasks/broken").exists());
    // A second try with the same name works.
    helm.new_space("broken").unwrap();
}

#[test]
fn complete_keeps_the_notes_of_the_agents() {
    let _env = crate::util::env_lock();
    let helm = Helm::new(&["web"], json!({ "web": { "copy": [".env"] } }));
    helm.new_space("notes").unwrap();
    let c = config().unwrap();
    let (rec, dir) = load_space(&c, Some("notes")).unwrap();
    // An empty .notes folder is not a loss.
    assert_eq!(losses(&rec, &dir).unwrap(), Vec::<serde_json::Value>::new());
    std::fs::write(dir.join(".notes/plan.md"), "the plan").unwrap();
    std::fs::create_dir_all(dir.join(".claude/skills/made-here")).unwrap();
    let lost = losses(&rec, &dir).unwrap();
    assert_eq!(
        lost,
        [
            json!({ "checkout": "", "what": ".notes/plan.md is not in a checkout" }),
            json!({ "checkout": "", "what": ".claude/skills/made-here/ is not in a checkout" })
        ]
    );
    std::fs::remove_file(dir.join(".notes/plan.md")).unwrap();
    std::fs::remove_dir(dir.join(".claude/skills/made-here")).unwrap();
    super::commands::complete_space(&ctx(Some("notes")), &CompleteArgs { check: false, close: false }).unwrap();
    assert!(!dir.exists());
    assert!(crate::util::has_ref(&helm.path("repos/web"), "refs/heads/notes"));
}

#[test]
fn a_refused_remove_keeps_the_copied_files() {
    let _env = crate::util::env_lock();
    let helm = Helm::new(&["web"], json!({ "web": { "copy": [".env"] } }));
    helm.new_space("keep").unwrap();
    let (rec, dir) = load_space(&config().unwrap(), Some("keep")).unwrap();
    std::fs::write(dir.join("web/work.txt"), "work").unwrap();
    let e = remove_checkout_dir(&rec["checkouts"][0], false).unwrap_err().to_string();
    assert!(e.contains("1 uncommitted changes"), "{e}");
    assert!(dir.join("web/.env").is_file());
    assert!(dir.join("web/work.txt").is_file());
}

#[test]
fn local_files_outside_the_checkout_are_never_touched() {
    let _env = crate::util::env_lock();
    // An older projects.json, written before the check: the copy entry leaves the checkout.
    let helm = Helm::new(&["web"], json!({ "web": { "copy": ["../../victim", ".env"] } }));
    std::fs::write(helm.path("victim"), "keep me").unwrap();
    helm.new_space("safe").unwrap();
    let (mut rec, dir) = load_space(&config().unwrap(), Some("safe")).unwrap();
    assert!(!dir.join("victim").exists());
    assert_eq!(rec["checkouts"][0]["local"], json!([".env"]));
    // A record with bad local entries: the removal leaves those paths alone.
    std::fs::write(dir.join("victim"), "keep me too").unwrap();
    rec["checkouts"][0]["local"] = json!(["../victim", helm.path("victim"), ".env"]);
    remove_checkout_dir(&rec["checkouts"][0], false).unwrap();
    assert!(!dir.join("web").exists());
    assert_eq!(std::fs::read_to_string(helm.path("victim")).unwrap(), "keep me");
    assert_eq!(std::fs::read_to_string(dir.join("victim")).unwrap(), "keep me too");
}

#[test]
fn add_keeps_a_change_made_by_another_command() {
    let _env = crate::util::env_lock();
    let helm = Helm::new(&["web"], json!({}));
    helm.new_space("fresh").unwrap();
    let c = config().unwrap();
    let (mut stale, dir) = load_space(&c, Some("fresh")).unwrap();
    update_space(&dir, |r| {
        r["herdr_space"] = json!("s7");
        Ok(())
    })
    .unwrap();
    add_checkout(&c, &mut stale, &dir, "web", Some("rev"), None, false).unwrap();
    let (rec, _) = load_space(&c, Some("fresh")).unwrap();
    assert_eq!(rec["herdr_space"], "s7");
    assert_eq!(rec["checkouts"].as_array().unwrap().len(), 2);
    assert_eq!(stale, rec);
    drop(helm);
}

#[test]
fn a_moved_checkout_is_still_checked_for_losses() {
    let _env = crate::util::env_lock();
    let helm = Helm::new(&["web"], json!({}));
    helm.new_space("moved").unwrap();
    let c = config().unwrap();
    let (mut rec, dir) = load_space(&c, Some("moved")).unwrap();
    std::fs::write(dir.join("web/new.txt"), "work").unwrap();
    // The record points to a path that is gone, as after a move of the tasks folder.
    rec["checkouts"][0]["path"] = json!(helm.path("old-tasks/moved/web").to_string_lossy());
    assert_eq!(losses(&rec, &dir).unwrap(), [json!({ "checkout": "web", "what": "1 uncommitted changes" })]);
    // A folder there that git does not know is a loss too.
    std::fs::remove_dir_all(dir.join("web")).unwrap();
    std::fs::create_dir(dir.join("web")).unwrap();
    std::fs::write(dir.join("web/file"), "x").unwrap();
    let lost = losses(&rec, &dir).unwrap();
    assert_eq!(lost.len(), 1, "{lost:?}");
}

#[test]
fn delete_counts_the_recorded_branch_and_keeps_a_branch_from_before() {
    let _env = crate::util::env_lock();
    let helm = Helm::new(&["web", "api"], json!({}));
    let api = helm.path("repos/api");
    // "mine" is a branch of api from before Helm.
    sh(&api, "git branch mine");
    helm.new_space("mine").unwrap();
    let c = config().unwrap();
    let (mut rec, dir) = load_space(&c, Some("mine")).unwrap();
    add_checkout(&c, &mut rec, &dir, "api", None, None, false).unwrap();
    let (rec, dir) = load_space(&c, Some("mine")).unwrap();
    let k = |repo: &str| rec["checkouts"].as_array().unwrap().iter().find(|k| k["repo"] == repo).unwrap().clone();
    assert_eq!(k("web")["made_branch"], true);
    assert_eq!(k("api")["made_branch"], false);

    // A commit on the space branch of web, then a switch to another branch.
    sh(&dir.join("web"), "git -c user.name=t -c user.email=t@t commit -q --allow-empty -m work && git switch -q -c elsewhere origin/main");
    let lost = losses(&rec, &dir).unwrap();
    assert!(lost.contains(&json!({ "checkout": "web", "what": "1 commits on mine not pushed" })), "{lost:?}");

    super::commands::delete_space(&ctx(Some("mine")), &DeleteArgs { yes: true, close: false }).unwrap();
    assert!(!crate::util::has_ref(&helm.path("repos/web"), "refs/heads/mine"));
    assert!(crate::util::has_ref(&api, "refs/heads/mine"), "a branch from before Helm stays");
}
