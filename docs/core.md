# The core

The core of `herdr-helm` manages projects, spaces, and the checkouts of a space.
The code is in `src/core/`. The core calls only `git`, `hc`, and `herdr-app`.
Every other service goes through a connector (see `src/connectors/mod.rs`).

| File | What it holds |
| --- | --- |
| `config.rs` | `projects.json`, the projects, their repos, the project checks, the name rules |
| `space.rs` | The space record, the list of spaces, the repos of a space or a folder |
| `checkout.rs` | The git worktrees of a space: add, state, changes, remove, losses |
| `agent_files.rs` | The `CLAUDE.md` and the skills of a space |
| `commands.rs` | The commands of the core |
| `herdr.rs` | Herdr (`herdr-app do`) and `hc` |

## The config

The config file is `projects.json` in the Herdr home (`paths::config_file()`).
The env var `HERDR_PROJECTS` sets another file. Inside a space, the program
uses the config that made the space (the `config` field of the record). It sets
`HERDR_PROJECTS` once at the start, so all reads agree.

| Key | What it is |
| --- | --- |
| `tasks_root` | The folder of the spaces. The default is `~/herdr/tasks`. |
| `projects` | The projects, by name. |
| `connectors` | The connector of each kind: a built-in name, or `{"command": "<program>"}`. |
| `code_host_settings`, `agent_settings`, `editor_settings`, `system_settings` | The settings of each connector. |
| `github`, `smart_rename` | Legacy blocks. The core reads them, but does not write them. |

`config()` returns the file as a `serde_json::Value`, with `tasks_root` as a
full path. Use `tasks_root(&c)` to get it as a `PathBuf`.

A project has these fields:

| Field | What it is |
| --- | --- |
| `root` | The folder that holds the git repos of the project. |
| `defaults` | The repos that a new space gets when you give no `--repo`. |
| `exclude` | The repos under the root that the project does not use. |
| `base` | The base of new branches, when a repo has it. Else the default branch of the repo. |
| `claude_md` | A `CLAUDE.md` template of the project. Else `templates/CLAUDE.md`. |
| `repos` | The settings of each repo: `copy`, `link` (lists), `setup`, `base` (text). |

`check_project` refuses a project that the core cannot use. `project update`
removes empty settings before the check.

Each `copy` and `link` entry must be a path inside the checkout: relative, and
with no `..` part (`inside_path`). `check_project` refuses an absolute path or a
`..`. The checkout code checks each entry again, because an older `projects.json`
can have a bad one. It also follows links: Helm never copies, links, or removes
a local file outside the checkout. A bad entry becomes a note, not an error.

`project add|update|remove` and `settings update` change `projects.json` under
its lock (`update_config`). They read the file again under the lock, so two
changes at the same time do not lose each other.

## The space record

A space is a folder in `tasks_root`. Its record is `.herdr/space.json`:

```json
{
  "name": "sso-login-times-out",
  "title": "SSO login times out",
  "project": "web",
  "goal": "SSO login times out",
  "created": 1767225600,
  "config": "/Users/me/.config/herdr-app/projects.json",
  "herdr_space": "s3",
  "checkouts": [
    { "repo": "web", "label": null, "folder": "web", "branch": "sso-login-times-out",
      "base": "origin/main", "path": "/Users/me/herdr/tasks/sso-login-times-out/web", "local": [".env"],
      "made_branch": true }
  ],
  "prs": []
}
```

A space name is 1-23 of `a-z 0-9 -`. It is also the folder and the branch of the
space. A label of an extra checkout is 1-8 of `a-z 0-9 -`. An extra checkout has
the folder `<repo>.<label>` and the branch `<space>-<label>`. It starts from the
branch of the main checkout of its repo.

`local` lists the files that `copy` and `link` put in the checkout. `status`,
`complete`, and `delete` do not count them as changes.

`made_branch` is `true` when Helm made the local branch of the checkout. `delete`,
`remove`, and the undo of a failed step delete only such a branch. A branch that
was there before, such as the branch of a PR that you had checked out, stays. A
record without `made_branch` is from an older Helm, so its branches stay too.

A command that changes a record that exists uses `update_space(dir, change)`.
It takes the lock `.herdr/space.json.lock`, reads the record again, applies the
change, and writes it. So a change by a mod or by another command is not lost.
Smart rename asks the agent with no lock. Then it puts only its own fields
(`title`, `titles`, `title_source`, `summary`, `work`, `smart_at`, `prs`) into
the record that is on disk at that time.

`write_json` writes each JSON file through its own temporary file in the same
folder, then renames it. A reader never sees half of a file. A checkout with
`"external": true` is the worktree of another tool. The space has only a link
to it (`link`), and the core never removes that worktree.

## The commands

All commands take `--json` and `--space <name>`. Without `--space`, a command
uses the space of the current folder. The name must be a valid space name, so
`--space ../x` or an absolute path is an error.

| Command | What it does |
| --- | --- |
| `projects` | Lists the projects. |
| `project show\|add\|update\|remove <name>` | Shows or changes a project. `update` takes `--data <JSON>`. |
| `settings [show\|update]` | Shows or changes the settings. `update` takes `--data <JSON>`. |
| `spaces` | Lists the spaces. |
| `new` | Makes a space with a checkout of each repo. All or nothing: when a step fails, the worktrees, their new branches, and the folder go. |
| `add <repo> [--as <label>]` | Adds a checkout to the space. |
| `status` | Shows the branch, the changes, and the push state of each checkout. |
| `changes` | Shows the changed files and the sync state of each repo of a space or a folder. |
| `remove <folder>` | Removes an extra checkout that is clean and merged. |
| `complete [--check] [--close]` | Removes the space when all work is pushed and no other files are in it. Only from Herdr. |
| `delete [--yes] [--close]` | Removes the space and the local branches that Helm made. Only from Herdr. |
| `skills sync` | Links the skills of every space again, both ways. |
| `refresh` | Writes the `CLAUDE.md` and the built-in skills of every space again. |

### settings

`settings update` accepts these keys: `tasks_root`, `connectors`,
`code_host_settings`, `agent_settings`, `editor_settings`, `system_settings`.
An object value merges into the object that is there. A `null` or `""` value
removes that key. The legacy keys also work:

- `github` (`host`, `org`, `refresh_minutes`) merges into `code_host_settings`.
  An empty field keeps the value that is there.
- `smart_rename` (`{"command": ...}`) sets `agent_settings.smart_command`.
  An empty command removes it.

With `--json`, the output is an object with `tasks_root`, `code_host_settings`
(`host`, `org`, `refresh_minutes`), `agent_settings` (`smart_command`,
`smart_default`, `usage`), `connectors` (what `projects.json` picks), `config`
(the path of the file), and `chosen` (what each connector can do). Without
`--json`, the output is text for a person: one line per setting, then one line
per connector kind.

`smart_default` comes from the chosen agent connector (`Agent::smart_default`).
It is `null` when the connector has no default command. The core does not know
the command of any one agent.

### new

`new --title <text>` makes the name from the title (`slugify`), and adds `-2`,
`-3` and so on when a space or a branch has that name. `mods/helm-task` shows
the same name while you type, so keep the two `slugify` functions the same.

`new --open` opens the space in Herdr. With `--agent agent` (the default) or the
legacy `--agent claude`, the first tab runs the command of the agent connector
(`start_argv`). With `--agent shell`, the first tab is a shell.

### complete and delete

Both commands refuse to run inside a terminal (`HC_TERMINAL_ID` is set). No flag
turns this check off. The check prevents a mistake. It is not a security limit: a
program can remove `HC_TERMINAL_ID` from its environment, for example with
`env -u HC_TERMINAL_ID`. The tests call `complete_space` and `delete_space`,
which do the work without the check.

When the recorded path of a checkout is gone, for example after a move of the
tasks folder, `losses` checks the folder of the checkout in the space folder
instead. A folder there that is not a git checkout is a loss, so `complete` stops.

`losses` lists what a removal would lose, as rows of `{"checkout", "what"}`:

- uncommitted changes and commits that are not pushed, per checkout.
- the files of the space folder that are not in a checkout and are not Helm's
  own (`extra_files`). These rows have `"checkout": ""`. Examples are the notes
  of the agents in `.notes/`, a skill that an agent made in `.claude/skills/`,
  and any other file or folder at the top of the space. Helm's own files are
  `.herdr/`, `CLAUDE.md`, `.claude/` (the built-in skills and the links to the
  skills of a checkout), and the checkout folders. An empty `.notes/` is not a
  loss. After 20 rows, one row gives the count of the other files.

`complete` removes nothing while there is a loss. So `complete` never deletes
the notes of an agent. Move the files into a checkout, remove them, or use
`delete`. `complete --check` prints `{"ok", "problems"}`, where `problems` is the
list of `losses`. When there is a problem, it exits with code 1. A successful
`complete` prints `{"ok": true, "kept_branches": [...]}`.

`delete` prints the same shape with and without `--yes`:

```json
{ "space": "sso-login-times-out", "deleted": false, "losses": [{ "checkout": "web", "what": "2 uncommitted changes" }] }
```

Without `--yes`, `deleted` is `false` and nothing changes. With `--yes`,
`deleted` is `true` and `losses` is what the removal lost.

The order of a removal keeps work when a step fails:

1. Without force (`complete`), each checkout must have no changes other than
   its local files. Only then do the local files go, and then
   `git worktree remove`. A refused checkout keeps its copied files.
2. With force (`delete`), `git worktree remove --force` removes the worktree.
   Only the linked files go first.
3. `--close` ends the terminals and closes the Herdr space only after the
   removal worked. When the removal fails, the agents keep running.

### refresh

The installer runs `refresh` after an update. The output is an object with a
list per space of what changed: `CLAUDE.md`, `skill: <name>`, and the skill
links. A space that fails gets `error: <message>` in its list, and the other
spaces continue.

## For the other parts

The other parts of `herdr-helm` use these functions from `crate::core`:

| Function | What it does |
| --- | --- |
| `config()`, `tasks_root(&c)`, `project(&c, name)`, `project_repos(p)` | Read the config. |
| `all_spaces(&c)` | Every space: `(record, folder)`, in the order of the names. |
| `load_space(&c, name)`, `save_space(&rec, dir)`, `record_path(dir)` | Read and write a record. Use `save_space` only for a new record. |
| `update_space(dir, change)` | Changes a record under its lock, on the record that is on disk now. |
| `prs_of(&rec)` | The PRs of a space (`prs`, or the legacy single `pr`). |
| `add_checkout(&c, &mut rec, dir, repo, label, branch, fetch)` | Adds a checkout and saves the record. |
| `checkout_status(&rec, &k)`, `repo_state(name, path)`, `repo_changes(name, path)` | The state of a checkout or a repo. |
| `space_repos(space, herdr_space, folder)` | The repos to show for a space or a folder. |
| `write_agent_files(&c, &rec, dir)`, `link_skills(&rec, dir)` | Write the agent files of a space. |
| `remove_checkout_dir(&k, force)`, `losses(&rec, dir)` | Remove a worktree in the safe order. What a removal would lose. |

`crate::util` has the helpers that are not special to the core:
`par_map(items, workers, f)` runs `f` on each item in parallel and keeps the
order, `locked(path, f)` runs `f` under the lock `<path>.lock`, `write_json`,
and `now_ms` and `now_secs`.
