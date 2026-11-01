# Architecture

Helm has three parts: a Herdr layout, a set of Herdr mods, and the `herdr-helm` program.

```
 Herdr window
 ├── layouts/helm.json        the layout: bar, grid of tiles, side panel, jobs block
 └── mods/helm-*              JavaScript views and background work
          │  exec
          ▼
 herdr-helm (Rust)            one program for the mods, for you, and for your agents
 ├── core                     projects, spaces, git worktrees, CLAUDE.md and skills
 ├── Herdr and hc             herdr-app do <command>, hc <command>
 └── connectors               code_host, agent, editor, system
          │
          ▼
 gh, claude, code, ps ...     or your own connector programs
```

## Herdr, hc, and herdr-core

Helm runs inside [Herdr](https://herdr.dev). Herdr is the terminal app, and its mods are
JavaScript views. herdr-core keeps the terminals alive, also when the window closes. `hc` is the
command line tool of herdr-core: it lists terminals, reads their labels, and sends them text.
`herdr-app do <command>` sends a command to the Herdr window, for example `space.new`.

Herdr is the host of Helm, not an outside service. So Herdr and `hc` are not connectors. The code
that calls them is in `src/core/herdr.rs`.

## The program

`herdr-helm` is a Rust program with no runtime. Run `herdr-helm --help`.

| Folder | What it holds |
| --- | --- |
| `src/cli.rs` | The commands and their flags (clap), and the dispatch to the command functions. |
| `src/core/` | Projects, spaces, checkouts, agent files, and Herdr. See [core.md](core.md). |
| `src/connectors/` | The four kinds of connectors, each with its trait, its built-ins, and the command form. See [connectors.md](connectors.md). |
| `src/commands/` | The commands that use a connector: `code_host.rs`, `agents.rs`, `host.rs` (open, system, connectors), and `install.rs`. |
| `src/paths.rs` | All paths, from the Herdr home. See [state.md](state.md). |
| `src/util.rs` | Process calls, git, JSON files, and output. |

Every command has text output for a person and `--json` output for the mods and agents.
`complete` and `delete` refuse to run inside a terminal (env `HC_TERMINAL_ID`), so that an agent
cannot end its own space. The mods run outside a terminal.

## The mods

| Mod | What it is |
| --- | --- |
| `helm-bar` | The top bar, the tiler of the grid, and the key commands of the layout. |
| `helm-tile` | The header of a tile: number, title, status, Make main, restart, full screen, close. |
| `helm-spaces` | The spaces window (`alt-space`, `alt-tab`). |
| `helm-menu` | The ⋯ menu: all actions with their keys. |
| `helm-github` | The PRs of the shown space, the top part of the GitHub popup. |
| `helm-prs` | The org's open PRs, the bottom part of the GitHub popup. |
| `helm-usage` | The agent usage popup, and the apps popup of the CPU and memory. |
| `helm-files` | The Files panel. |
| `helm-changes` | The Changes panel. |
| `helm-task` | The New space, Complete, and Delete dialogs. |
| `helm-projects` | Helm Settings: projects, settings, and connectors. |
| `helm-jobs` | The background work. It has no view. |

`mods/_shared/` holds code that more than one mod uses. A mod cannot import from the folder of
another mod, so the installer copies these files into each mod:

- `helm.js`: the calls to `herdr-helm` (`helm`, `helmSlow`), the background job helper
  (`background`), `keep` (a shared write only of a changed value), the agent checks (`isAgent`,
  `restartAgent`), the capability check (`can`), and `OVERLAYS`, the one table of overlay sizes,
  with `overlay` to open one.
- `space-actions.js`: the actions of a space, as buttons for the bar and the spaces window.
- `ui.js`: small parts for the views. `awake` (Herdr is in front), `location` (the focused
  folder and its machine), `place` (the folder of a space), `ago` and `agoText` (times),
  `shortHome` (a path with `~`), `programIcon`, and `checkBox`.

## Background work and shared values

`helm-jobs` runs in the `jobs-run` block of the layout. The block shows at 0 px and is not hidden,
because Herdr starts a mod only when its block shows. The mods share data through Herdr's shared
values. Each write makes every mod render again, so a job writes only when a value changed.

| Shared value | Written by | What it holds |
| --- | --- | --- |
| `helm.connectors` | helm-jobs, every minute, and Helm Settings after a change | The chosen connectors and their capabilities |
| `helm.update` | helm-jobs, every hour (herdr-helm asks GitHub at most every few hours) | A newer Helm release: `current`, `latest`, `available`, `url`. The ⋯ button shows a dot. |
| `helm.updating` | helm-bar | True while the update that the person asked for runs |
| `helm.projects`, `helm.spaces` | helm-jobs, every 20 s. helm-task and Helm Settings write `helm.projects` after a read | The projects and the project spaces |
| `helm.project.<name>` | helm-task, Helm Settings | `herdr-helm project show` of a project, so that the dialogs show it at once |
| `helm.info` | helm-bar, every 30 s | `herdr-helm info` of each space, for the spaces window |
| `github.cache` | helm-jobs, on a switch and every 2 min | The repos and PRs of each shown space |
| `prs.data`, `prs.error`, `prs.loading` | helm-jobs, every few minutes | The org's open PRs, the last error, and whether a read runs |
| `agent.usage`, `agent.usage.error` | helm-jobs, every 5 min | The agent's usage limits, and the last error |
| `smart.running` | helm-jobs | The spaces that Smart rename works on |
| `helm.order`, `helm.zoom` | helm-bar, helm-tile | Tile order, and the full screen tile of each space |
| `helm.activity` | helm-bar | The last activity of each space, for the switcher |
| `helm.tiles` | helm-bar | The tile numbers of the shown space, for the GitHub popup |
| `helm.drag`, `helm.move` | helm-tile, helm-bar | A tile that is dragged, and a swap that the tiler must do |
| `helm.switcher.step` | helm-bar | An alt-tab press while the spaces window starts |
| `helm.switcher.closed` | helm-spaces | The session of the spaces window that closed |
| `changes.count` | helm-changes | The count on the Changes button |

The mods also read two values of Herdr: `layout` (the name of the shown layout) and
`workspace.panels` (the side panels that show).

The timestamps `at` in `github.cache` and `agent.usage` change at each read. helm-jobs writes a
read that brings only a new `at` when the person asked for it, or when the kept `at` is 10 minutes
old.

A mod asks for fresh data with a request value: `github.refresh` (helm-github), `prs.refresh`
(helm-prs), `agent.usage.refresh` (helm-usage, Helm Settings), or `smart.request` (the space
actions and helm-bar).

Background reads pause while another app is in front, and catch up when Herdr is in front again.
Herdr's `exec` stops a process after 30 seconds. Long commands, such as `herdr-helm prs` or
`herdr-helm new`, run in the background through `helmSlow` in `helm.js`, and a commit or a push
of the Changes panel through `background`. Their output goes to `helm/state/jobs/`, or to a temp
folder on a remote machine.

## What the mods read from herdr-helm

The mods read the `--json` output of `herdr-helm`. Some fields have a special use:

- `prs`: each PR can have `queue_url`, the link of its merge queue. Without it, the mods show no
  link. No mod makes a URL of the code host.
- `settings`: `tasks_root`, the tasks folder. The New space dialog does not offer a folder in it
  for a plain space.
- `spaces`: `path`, the folder of each space. The Complete and Delete dialogs show it.
