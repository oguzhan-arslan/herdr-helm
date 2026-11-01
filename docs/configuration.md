# Configuration

All settings are in `projects.json` in the Herdr home (`$HERDR_APP_HOME/projects.json`, by default
`~/.config/herdr-app/projects.json`). The env var `HERDR_PROJECTS` points Helm to another file.

Helm Settings (`cmd-shift-,`) changes this file for you, and `herdr-helm project` and
`herdr-helm settings` do the same from a terminal. The installer never changes it.
[projects.example.json](../projects.example.json) is a complete example.

## The whole file

```json
{
  "tasks_root": "~/herdr/tasks",
  "projects": {
    "<name>": {
      "root": "~/code",
      "exclude": ["<repo>"],
      "defaults": ["<repo>"],
      "base": "origin/main",
      "repos": {
        "<repo>": {
          "copy": ["<file>"],
          "link": ["<file>"],
          "setup": "<shell command>",
          "base": "origin/develop"
        }
      },
      "claude_md": "<template path>"
    }
  },
  "connectors": { "code_host": "github", "agent": "claude-code", "editor": "vscode" },
  "code_host_settings": { "host": "github.com", "org": "<org>", "refresh_minutes": 5 },
  "agent_settings": { "usage": false, "smart_command": "claude -p --model haiku" },
  "editor_settings": {},
  "system_settings": {}
}
```

## Spaces

| Key | What it does |
| --- | --- |
| `tasks_root` | The folder of the spaces. Each space gets `<tasks_root>/<space>/`. Default `~/herdr/tasks`. |

A space can have more than one checkout of a repo, for parallel agents. The extra checkout is
`<repo>.<label>/` on the branch `<space>-<label>`.

## Projects

A **project** is a root folder whose subfolders are git repos.

| Key | What it does |
| --- | --- |
| `root` | The folder that holds the repos. Every git repo directly under it is part of the project. |
| `exclude` | Repos that are not part of the project. |
| `defaults` | The repos that the New space dialog picks first. |
| `base` | Where a new space branch starts, in each repo that has this ref. Default: the remote's default branch. |
| `repos.<repo>.copy` | Files that each new checkout gets as a copy of the root checkout's file, for example `.env.local`. |
| `repos.<repo>.link` | Files that each new checkout gets as a symlink to the root checkout's file. |
| `repos.<repo>.setup` | A shell command that runs in each new checkout, for example `npm install`. |
| `repos.<repo>.base` | The base of this repo. It comes before the project's `base`. |
| `claude_md` | Your own template for the `CLAUDE.md` of a space. Default: `templates/CLAUDE.md`. |

- `copy` and `link` paths are relative to the repo. Helm adds them to the checkout's
  `.git/info/exclude`, so they never count as changes.
- `setup` gets `HERDR_ROOT_PATH`, `HERDR_WORKTREE_PATH`, and `HERDR_SPACE` in its environment.

### Skills in a space

Claude Code reads `~/.claude/skills` and the `.claude/skills` of the folder that it starts in. It
does not read the skills of a parent or a child folder. So Helm shares the skills between a space
and its checkouts:

- Each checkout's own skills get a link in the space folder.
- The space's skills, which are the built-in ones from `templates/skills`, get a link in each
  checkout. The link is in the repo's `.git/info/exclude`, so it is never a change.
- A name that is already there stays.

`herdr-helm skills sync` makes the links again for all spaces. A running agent sees new skills
after a restart.

## Connectors and their settings

`connectors` picks the connector of each kind. See [connectors.md](connectors.md).

### code_host_settings (github)

| Key | Default | What it does |
| --- | --- | --- |
| `host` | `github.com` | The host that `gh` uses, for example `your-company.ghe.com`. |
| `org` | none | The org of the open PRs. The PR features need it. |
| `refresh_minutes` | `5` | Minutes between two reads of the org's PRs. |
| `merge_method` | the first that the repo allows | The merge method of Add to queue on a repo without a merge queue. |
| `stacks` | `false` | Shows the stack of each PR. The public GitHub schema does not have the stack fields, so this is off by default. |

An older `github` block with the same keys still works. Helm reads it for each key that
`code_host_settings` does not have.

### agent_settings (claude-code)

| Key | Default | What it does |
| --- | --- | --- |
| `usage` | `false` | Shows the usage limits in the bar. It works only on macOS. See the note below. |
| `smart_command` | `claude -p --model haiku` | The command of Smart rename. It reads a prompt on stdin and prints only a JSON object. |

**Usage limits.** With `usage` on, Helm reads the Claude Code sign-in from the macOS keychain
(`security find-generic-password -s "Claude Code-credentials"`). It then asks Anthropic's usage
endpoint, the same one that Claude Code's `/usage` uses, for the limits of your plan. The token
stays in a shell variable and goes to curl on stdin, so it never shows in a process list. Nothing
else leaves your machine. Because Helm reads a secret, this is off until you turn it on in Helm
Settings → Settings, or set `"usage": true`.

**Smart rename.** Helm sends all the facts of a space to the command: the checkouts with their
commits and changed files, the PRs, and the agent sessions with their titles and up to 12 of their
prompts. The command returns a title, a summary, and the separate pieces of work. The default runs
with no tools and keeps no session.

### editor_settings (vscode)

| Key | Default | What it does |
| --- | --- | --- |
| `code` | none | The `code` program: a path (`~/` works), or a program name on the PATH, for example `code-insiders`. Without it, Helm looks on the PATH, then in the PATH of a login shell, then in the app bundle on macOS. |

## Paths

| Env var | What it changes |
| --- | --- |
| `HERDR_APP_HOME` | The Herdr home. Helm installs into it and keeps all its state under `helm/`. A second Herdr home gets its own Helm, but `~/.local/bin/herdr-helm` links to the last one installed. |
| `HERDR_PROJECTS` | The config file. |
| `HELM_TEMPLATES` | The templates folder, for development. |

See [state.md](state.md) for every file that Helm writes.
