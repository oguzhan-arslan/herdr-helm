//! The command line: the commands and their flags, and the dispatch to the command functions.
//! The mods and the agents in a space call these commands, so keep the names, the flags and
//! the `--json` output stable. Each command is a function in the module of its area.

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "herdr-helm", version, about = "Helm for Herdr: spaces of git worktrees, agents, and the services around them")]
pub struct Cli {
    /// JSON output.
    #[arg(long, global = true)]
    pub json: bool,
    /// The space by name (default: the one this folder is in).
    #[arg(long, global = true)]
    pub space: Option<String>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// What every command gets besides its own arguments.
pub struct Ctx {
    pub json: bool,
    pub space: Option<String>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// List the projects.
    Projects,
    /// Show, add, update, or remove a project.
    Project(ProjectArgs),
    /// Show or update the settings (tasks folder, connectors and their settings).
    Settings(SettingsArgs),
    /// List the spaces.
    Spaces,
    /// Make a space with a checkout per repo.
    New(NewArgs),
    /// Add a checkout of a repo to the space.
    Add(AddArgs),
    /// Branch, changes and push state of each checkout.
    Status,
    /// Changed files and sync state of each repo of a space or folder.
    Changes(SpaceOrFolder),
    /// Branch, changes, and PR of each repo of a space or folder.
    Github(SpaceOrFolder),
    /// Remove an extra checkout that is clean and merged.
    Remove(RemoveArgs),
    /// Remove the space when all work is pushed (Herdr only).
    Complete(CompleteArgs),
    /// Remove the space and the local branches that Helm made (Herdr only).
    Delete(DeleteArgs),
    /// The agents in this space.
    Agents,
    /// Start a handoff agent in its own tab.
    Spawn(SpawnArgs),
    /// Send a message to an agent in this space.
    Send(SendArgs),
    /// Wait for the next message or a child that stops.
    Wait,
    /// The open PRs of the org, in sections by what they need from you.
    Prs,
    /// Review or fix a PR in its own space, or add it to the merge queue.
    Pr(PrArgs),
    /// Record agent sessions, or resume those a shutdown ended.
    Sessions(SessionsArgs),
    /// Link the skills of every space both ways (space and checkouts) again.
    Skills(SkillsArgs),
    /// Write the CLAUDE.md and the built-in skills of every space again (after an update).
    Refresh,
    /// Restart the agent of a terminal in the same session (after an update).
    Restart(RestartArgs),
    /// Smart rename: an agent names the space after all its work; or give it --title.
    Rename(RenameArgs),
    /// All that is known about the space: PRs, checkouts, sessions, titles.
    Info,
    /// The agent's usage limits (when its connector can read them).
    Usage,
    /// Open a file, a diff, or a folder in the editor.
    Open(OpenArgs),
    /// CPU and memory, or the apps that use the most (--apps).
    System(SystemArgs),
    /// The chosen connector of each kind and what it can do.
    Connectors,
    /// Install Helm into the Herdr home, or update it (safe to run again).
    Install(InstallArgs),
    /// Install the newest release of the source repo, or only check for one (--check).
    Update(UpdateArgs),
    /// Remove Helm from the Herdr home; projects.json and the spaces stay.
    Uninstall(InstallArgs),
}

#[derive(Args)]
pub struct ProjectArgs {
    #[arg(value_parser = ["show", "add", "update", "remove"])]
    pub action: String,
    pub name: String,
    /// add: the folder that holds the repos.
    #[arg(long)]
    pub root: Option<String>,
    /// add: a default repo (repeat).
    #[arg(long = "default")]
    pub default: Vec<String>,
    /// update: JSON with the fields to replace, e.g. {"defaults": ["web"]}.
    #[arg(long)]
    pub data: Option<String>,
}

#[derive(Args)]
pub struct SettingsArgs {
    #[arg(value_parser = ["show", "update"], default_value = "show")]
    pub action: String,
    /// update: JSON, e.g. {"code_host_settings": {"refresh_minutes": 10}}.
    #[arg(long)]
    pub data: Option<String>,
}

#[derive(Args)]
pub struct NewArgs {
    /// The space name; default: made from --title.
    pub name: Option<String>,
    /// What the space is for, in a few words: its title, and its name and branch.
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub project: String,
    /// A repo to check out (repeat; default: the project's defaults).
    #[arg(long = "repo")]
    pub repo: Vec<String>,
    /// What the space is for: goes into CLAUDE.md, and to the first agent when given.
    #[arg(long)]
    pub goal: Option<String>,
    /// Open it in Herdr.
    #[arg(long)]
    pub open: bool,
    /// The first tab: an agent (default) or a shell.
    #[arg(long, value_parser = ["claude", "agent", "shell"], default_value = "agent")]
    pub agent: String,
    /// Report each repo on stderr as it starts.
    #[arg(long)]
    pub progress: bool,
}

#[derive(Args)]
pub struct AddArgs {
    pub repo: String,
    /// An extra checkout for parallel work, e.g. --as api.
    #[arg(long = "as")]
    pub label: Option<String>,
}

#[derive(Args)]
pub struct SpaceOrFolder {
    /// The Herdr space id (a project space).
    #[arg(long)]
    pub herdr_space: Option<String>,
    /// Else: the repo at this folder, or the repos under it.
    #[arg(long)]
    pub folder: Option<String>,
}

#[derive(Args)]
pub struct RemoveArgs {
    /// Its folder, e.g. web.api.
    pub checkout: String,
}

#[derive(Args)]
pub struct CompleteArgs {
    /// Only say what blocks it.
    #[arg(long)]
    pub check: bool,
    /// Also close the Herdr space.
    #[arg(long)]
    pub close: bool,
}

#[derive(Args)]
pub struct DeleteArgs {
    /// Really delete; without it, list what is lost.
    #[arg(long)]
    pub yes: bool,
    /// Also close the Herdr space.
    #[arg(long)]
    pub close: bool,
}

#[derive(Args)]
pub struct SpawnArgs {
    /// A short name, e.g. api.
    pub name: String,
    pub task: String,
    /// The checkout it works in, e.g. web.api.
    #[arg(long)]
    pub checkout: Option<String>,
    /// No tab of its own: open it with peek.
    #[arg(long)]
    pub no_tab: bool,
}

#[derive(Args)]
pub struct SendArgs {
    /// Its name or address.
    pub agent: String,
    pub text: String,
}

#[derive(Args)]
pub struct PrArgs {
    #[arg(value_parser = ["review", "fix", "enqueue"])]
    pub action: String,
    /// The repo name on the code host, e.g. web.
    pub repo: String,
    pub number: u64,
    /// fix: what is wrong, e.g. 'CI failed, 2 open threads'.
    #[arg(long)]
    pub reasons: Option<String>,
}

#[derive(Args)]
pub struct SessionsArgs {
    #[arg(value_parser = ["save", "resume"])]
    pub action: String,
}

#[derive(Args)]
pub struct SkillsArgs {
    #[arg(value_parser = ["sync"])]
    pub action: String,
}

#[derive(Args)]
pub struct RestartArgs {
    /// Its address, e.g. t7.
    #[arg(long)]
    pub terminal: String,
}

#[derive(Args)]
pub struct RenameArgs {
    /// Your own title.
    #[arg(long)]
    pub title: Option<String>,
    /// The Herdr space id: a plain space too.
    #[arg(long)]
    pub herdr_space: Option<String>,
}

#[derive(Args)]
pub struct OpenArgs {
    /// A file to open.
    pub path: Option<String>,
    /// The diff of one changed file against HEAD: REPO STATUS:PATH (status D: deleted).
    #[arg(long, num_args = 2, value_names = ["REPO", "STATUS:PATH"])]
    pub change: Vec<String>,
    /// A new editor window on REPO with a diff per changed file: REPO STATUS:PATH...
    /// Runs in the background, because one editor call per file takes long.
    #[arg(long, num_args = 2.., value_names = ["REPO", "STATUS:PATH"])]
    pub changes: Vec<String>,
    /// Internal: do the --changes work now (the background run).
    #[arg(long, hide = true)]
    pub now: bool,
}

#[derive(Args)]
pub struct SystemArgs {
    /// The apps that use the most CPU and memory, instead of the totals.
    #[arg(long)]
    pub apps: bool,
}

#[derive(Args)]
pub struct UpdateArgs {
    /// Only check: is a newer release there? Asks GitHub at most every few hours.
    #[arg(long)]
    pub check: bool,
    /// With --check: ask GitHub now, not the kept answer.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct InstallArgs {
    /// Show what would change, change nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// The repo to install from (default: the folder of this program, two levels up).
    #[arg(long)]
    pub from: Option<String>,
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let ctx = Ctx { json: cli.json, space: cli.space };
    crate::core::config_of_space();
    use crate::{commands, core};
    match &cli.cmd {
        Cmd::Projects => core::cmd_projects(&ctx),
        Cmd::Project(a) => core::cmd_project(&ctx, a),
        Cmd::Settings(a) => core::cmd_settings(&ctx, a),
        Cmd::Spaces => core::cmd_spaces(&ctx),
        Cmd::New(a) => core::cmd_new(&ctx, a),
        Cmd::Add(a) => core::cmd_add(&ctx, a),
        Cmd::Status => core::cmd_status(&ctx),
        Cmd::Changes(a) => core::cmd_changes(&ctx, a),
        Cmd::Remove(a) => core::cmd_remove(&ctx, a),
        Cmd::Complete(a) => core::cmd_complete(&ctx, a),
        Cmd::Delete(a) => core::cmd_delete(&ctx, a),
        Cmd::Skills(a) => core::cmd_skills(&ctx, a),
        Cmd::Refresh => core::cmd_refresh(&ctx),
        Cmd::Github(a) => commands::code_host::cmd_github(&ctx, a),
        Cmd::Prs => commands::code_host::cmd_prs(&ctx),
        Cmd::Pr(a) => commands::code_host::cmd_pr(&ctx, a),
        Cmd::Agents => commands::agents::cmd_agents(&ctx),
        Cmd::Spawn(a) => commands::agents::cmd_spawn(&ctx, a),
        Cmd::Send(a) => commands::agents::cmd_send(&ctx, a),
        Cmd::Wait => commands::agents::cmd_wait(&ctx),
        Cmd::Sessions(a) => commands::agents::cmd_sessions(&ctx, a),
        Cmd::Restart(a) => commands::agents::cmd_restart(&ctx, a),
        Cmd::Rename(a) => commands::agents::cmd_rename(&ctx, a),
        Cmd::Info => commands::agents::cmd_info(&ctx),
        Cmd::Usage => commands::agents::cmd_usage(&ctx),
        Cmd::Open(a) => commands::host::cmd_open(&ctx, a),
        Cmd::System(a) => commands::host::cmd_system(&ctx, a),
        Cmd::Connectors => commands::host::cmd_connectors(&ctx),
        Cmd::Install(a) => commands::install::cmd_install(&ctx, a),
        Cmd::Update(a) => commands::update::cmd_update(&ctx, a),
        Cmd::Uninstall(a) => commands::install::cmd_uninstall(&ctx, a),
    }
}
