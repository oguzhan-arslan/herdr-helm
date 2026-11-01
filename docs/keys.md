# Keys

All keys are in `keys.json` in the Herdr home. Helm uses two sections there:

- `layout:helm`: the keys of the helm layout.
- `mod:helm-spaces`: the keys of the spaces window while it has the keyboard.

Herdr's own keys are in `global` and `terminal`. Helm does not touch them.

## The default keys

| Key | Action |
| --- | --- |
| `cmd-n`, `ctrl-n` | New space dialog |
| `alt-space` | The spaces window (press again to close it) |
| `alt-tab` | The last space (a quick press), or the spaces window (hold `alt`) |
| `alt-shift-tab` | The same, in the other direction |
| `shift-f2` | Rename the space |
| `cmd-shift-f2` | Smart rename the space |
| `cmd-d`, `cmd-shift-d` | A new terminal, as a new tile |
| `cmd-shift-t` | The new agent menu |
| `cmd-]`, `cmd-[` | The keyboard to the next or the previous tile |
| `cmd-1` … `cmd-9` | The keyboard to tile N. The main tile is 1. In full screen, tile N in full screen. |
| `cmd-shift-m` | Make the focused tile the main tile |
| `cmd-shift-enter` | Full screen for the focused tile, or the grid again |
| `cmd-shift-r` | Restart the agent in the focused tile, in the same session |
| `cmd-w` | Close the tab of the focused tile (asks first) |
| `cmd-shift-e` | Files panel |
| `cmd-shift-c` | Changes panel |
| `cmd-shift-g` | GitHub popup |
| `cmd-shift-u` | Agent usage popup |
| `cmd-shift-,` | Helm Settings |
| `cmd-k` | The ⋯ menu |

In helm, a tile is not a pane of a split tab, so pane splits do not work. `cmd-d` starts a new
tile.

`keys/helm.json` in this repo has the defaults. `tests/mods.rs` checks that each `helm-*` command
names a mod that exists.

## Change a key for yourself

Edit the `layout:helm` section of your `keys.json`. The installer keeps your change:

- A key that you changed or removed stays as you left it.
- A key that still has its old default follows the repo on the next install.
- A default that the repo drops goes away, unless you changed it.

The installer knows the old defaults from `helm/install.json`.

## Add a key to Helm

1. Add the key to `keys/helm.json`. A key runs a command. That is a Herdr command such as
   `tab.new`, or a command of the bar such as `helm-bar.zoom`.
2. For a new bar command, handle it in `command()` or in the command loop of
   `mods/helm-bar/main.js`.
3. To show it in the ⋯ menu, add a row with the same command to `items()` in
   `mods/helm-menu/main.js`. The menu reads the key from `keys.json`.
4. Run `./install.sh`.
