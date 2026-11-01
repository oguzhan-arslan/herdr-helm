# State and paths

Everything derives from the Herdr home: `$HERDR_APP_HOME`, by default `~/.config/herdr-app`. A
second Herdr home gets its own mods, program, and state. The link `~/.local/bin/herdr-helm` is
shared: it points to the Herdr home of the last install.

## What Helm keeps

| Path | What it is |
| --- | --- |
| `<home>/projects.json` | Your projects and settings. Only Helm Settings and `herdr-helm` write it. |
| `<home>/helm/bin/herdr-helm` | The installed program. The mods call it here. |
| `<home>/helm/templates/` | The `CLAUDE.md` and the built-in skills of a space. |
| `<home>/helm/state/sessions.json` | Which agent session runs in which terminal, for the resume after a restart. |
| `<home>/helm/state/space-titles.json` | The titles of plain spaces, which have no record of their own. |
| `<home>/helm/state/smart/` | The work folder of Smart rename. |
| `<home>/helm/state/jobs/` | Output of the background jobs of the mods. Files older than one day go away. |
| `<home>/helm/state/diff/` | The HEAD side of the diffs that the editor shows. |
| `<home>/helm/state/update.json` | The last answer of the update check, so that Helm asks GitHub at most every few hours. |
| `<home>/helm/state/source/` | The files that an install without a clone takes from inside the program. |
| `<home>/helm/state/update/` | The download of an update, while it installs. |
| `<home>/helm/install.json` | What the last install did: the version, the repo, the mods, and the keys. |
| `~/.local/bin/herdr-helm` | A link to the program, so that you and your agents can run it. Put `~/.local/bin` on your PATH. The mods do not need it. |
| `<tasks_root>/<space>/` | Your spaces, by default under `~/herdr/tasks`. They are your work, not Helm's state. |
| `<tasks_root>/<space>/.herdr/space.json` | The record of a space: its checkouts, PRs, title, and summary. |

## What the installer changes, and what it keeps

`./install.sh` runs `herdr-helm install --from <repo>`. Before it writes anything, it checks the
Herdr home, `keys.json`, and the repo. A failed check changes nothing.

| Path | What happens |
| --- | --- |
| `mods/helm-*` | Replaced with the repo's mods, as copies, because Herdr does not reload a mod behind a symlink. Each mod also gets the files of `mods/_shared`. A Helm mod that the repo no longer has goes away. Files that Herdr writes there stay. |
| `layouts/helm.json` | The repo's structure. The panel's state and size, the tiles, and the keys that you added on the root (a theme, colors, fonts) stay. |
| `keys.json` | Only the `layout:helm` and `mod:helm-spaces` sections, see [keys.md](keys.md). |
| `helm/` | The program, the templates, and `install.json`. |
| `~/.local/bin/herdr-helm` | A link to `helm/bin/herdr-helm`. |
| `projects.json` | Never touched. |

The first install also switches Herdr to the helm layout. `--dry-run` lists the changes and
writes nothing.

## Uninstall

`./install.sh --uninstall` switches Herdr to its main layout. It then removes the Helm mods, the
helm layout, the Helm key sections, the links, and `helm/`. `projects.json` and your spaces stay.
