# Connectors

Helm reaches every outside tool through a connector. The core and the mods never call a service
directly. This lets you replace a tool without a change to Helm, and lets you add support for a
new tool in any language.

## The four kinds

| Kind | What Helm uses it for | Built in | Default |
| --- | --- | --- | --- |
| `code_host` | The PR of each branch, the open PRs of the org, review, fix, merge queue | `github` | `github` |
| `agent` | Start an agent, record and resume sessions, restart, Smart rename, usage limits | `claude-code` | `claude-code` |
| `editor` | Open a file, a diff against HEAD, a window with many diffs | `vscode` | `vscode` |
| `system` | CPU and memory in the bar, the apps that use the most | `macos`, `linux` | the one of your OS |

Each kind has its own page with all its methods:

- [connectors-code-host.md](connectors-code-host.md)
- [connectors-agent.md](connectors-agent.md)
- The editor and system kinds are small. This page describes them, see below.

## Choose a connector

Set `connectors` in `projects.json`, or use Helm Settings → Connectors:

```json
{
  "connectors": {
    "code_host": "github",
    "agent": { "command": "~/bin/my-agent-connector" },
    "editor": "vscode",
    "system": "none"
  }
}
```

- A string picks a built-in connector by name.
- `"none"` turns the kind off.
- `{"command": "<program>"}` picks an external program. The value is a command line, so it can
  have arguments, for example `"python3 ~/helm/gitlab.py"`. Helm splits it into words as a shell
  does, but it does not start a shell. A word that is `~` or starts with `~/` gets your home
  folder. Helm does not expand `$VARIABLES` or `~user`.
- A kind that is not in `connectors` uses its default.

The settings of a connector are in `<kind>_settings`, for example `code_host_settings`. Helm sends
this block to an external connector with each request.

Run `herdr-helm connectors` to see the chosen connector of each kind and what it can do.

## Capabilities: what the screen shows

Each connector names its capabilities. The mods read them (helm-jobs puts them in the shared
value `helm.connectors`) and hide every part that the connector cannot serve. Nothing breaks when a
kind is `none`. The part just does not show.

The commands also check the capability before they call the connector. A command whose
capability is missing stops with a message that names the connector and the capability. One
command does not stop: `github` without `branch_prs`, or without a code host, gives the repos
with `"pr": null`, because the mods show the repos.

| Command | Capability that it needs |
| --- | --- |
| `github` | `code_host` `branch_prs` (else `"pr": null`) |
| `prs` | `code_host` `org_prs` |
| `pr review`, `pr fix`, `pr enqueue` | `code_host` `review`, `fix`, `merge_queue` |
| `open FILE` | `editor` `open` |
| `open --change` | `editor` `diff` |
| `open --changes` | `editor` `window` and `diff` |
| `system`, `system --apps` | `system` `usage`, `apps` |
| `usage` | `agent` `usage` |

| Kind | Capability | What shows only with it |
| --- | --- | --- |
| code_host | `branch_prs` | The PR of each branch: bar, spaces window, GitHub popup (top part) |
| code_host | `org_prs` | The org's open PRs in the GitHub popup, and the PR badge |
| code_host | `review`, `fix` | The Review and Fix buttons on a PR |
| code_host | `merge_queue` | The Add to queue button |
| agent | `start` | The agent choice in the New space dialog |
| agent | `sessions`, `resume` | The session record, and the resume after a restart |
| agent | `restart` | The restart button of a tile, and the restart key |
| agent | `smart_rename` | Smart rename |
| agent | `usage` | The usage icon in the bar and its popup |
| editor | `open` | A click on a file in the Files panel |
| editor | `diff` | A click on a file in the Changes panel |
| editor | `window` | Open all in the Changes panel |
| system | `usage` | CPU and memory in the bar |
| system | `apps` | The apps popup |

## The JSON contract of an external connector

Helm starts the program once for each call. The program reads one JSON request on stdin and
writes one JSON result on stdout.

```json
{"kind": "editor", "op": "open", "args": {"path": "/abs/file.rs"}, "settings": {}}
```

- `kind` is the kind, `op` is the method, and `args` holds its arguments.
- `settings` is the `<kind>_settings` block of `projects.json`, or `{}`.
- The result is the JSON value of the method. An empty stdout counts as `null`.
- A non-zero exit is an error. Helm shows all of stderr, without the blank lines at the start
  and the end, as the message: `<kind> connector <op>: <stderr>`. With an empty stderr, the
  message is the exit code.
- Every kind has the op `describe`. It returns `{"name": "...", "capabilities": ["..."]}`.
  Helm asks it once per process for each kind and keeps the reply. A failed `describe` counts
  as no capabilities.
- Each call must finish in 60 seconds. Then Helm stops the program and its child processes,
  and the call fails with a message that names the program.

A method that the connector does not support must not be in its capabilities. Helm then does not
call it.

### Editor ops

| Op | Args | Result | Capability |
| --- | --- | --- | --- |
| `open` | `{"path"}` | anything | `open` |
| `diff` | `{"old", "new", "reuse"}` | anything | `diff` |
| `window` | `{"folder"}` | anything | `window` |

- `diff` shows the file `old` against the file `new`. Helm writes the HEAD side of a change to a
  file in `$HERDR_APP_HOME/helm/state/diff/`. A deleted file has an empty file as its new side.
- `reuse` is true when the diff goes into the window that `window` just opened.
- For Open all, Helm calls `window` once, waits 2 seconds, then calls `diff` for each file. This
  runs in the background.

The built-in `vscode` connector finds `code` in this order:

1. `editor_settings.code` in `projects.json`. The value is a path (`~/` works), or a program
   name on the PATH, for example `"code-insiders"`. Helm uses it only when the file is there.
2. The PATH, and then `~/.local/bin`.
3. The PATH of a login shell (`zsh -lc`, then `bash -lc`), because Herdr starts mods without it.
4. The app bundle on macOS, in `/Applications` or `~/Applications`.

Without `code`, the connector has no capabilities.

### System ops

| Op | Args | Result | Capability |
| --- | --- | --- | --- |
| `usage` | `{}` | `{"cpu", "mem", "total"}` | `usage` |
| `apps` | `{}` | `{"cores", "total", "apps": [{"name", "cpu", "kb", "count"}]}` | `apps` |

- `cpu` and `mem` are percentages in use. `total` is the memory size in bytes.
- In `apps`, `cpu` is the CPU percentage of the app (100 is one core), `kb` its resident memory,
  and `count` the number of its processes. Helm ranks the apps itself.
- `herdr-helm system` prints a line for a person, and `--json` prints the JSON. The mods use
  `--json`.

## Write a connector

A connector is any program that follows the contract. This example is an editor connector for
Zed, in shell, with `jq`:

```sh
#!/bin/sh
# An editor connector for Zed. Save it as ~/bin/zed-connector and make it executable.
req=$(cat)
op=$(printf %s "$req" | jq -r .op)
case "$op" in
  describe) echo '{"name": "zed", "capabilities": ["open"]}' ;;
  open) zed "$(printf %s "$req" | jq -r .args.path)" && echo null ;;
  *) echo "zed-connector: no op $op" >&2; exit 2 ;;
esac
```

Then set `"editor": {"command": "~/bin/zed-connector"}`. The Changes panel no longer opens diffs,
because the connector does not name `diff`.

To add a built-in connector, add a file next to the others in `src/connectors/<kind>/`, implement
the trait of the kind in its `mod.rs`, and add its name to `current()`. See
[development.md](development.md). The command form of all kinds is in
`src/connectors/command.rs`: one `Command` struct calls the ops, and each trait maps its methods to
ops with the same name.
