# The helm layout in detail

This page describes how each part of the layout behaves. The [README](../README.md) gives the
short version.

## Tiles

- Each terminal of the shown space is a tile. The main tile takes the left half. The other tiles
  share the right half, in more rows than columns, with at most one row more.
- Drag a border to resize. Each title bar starts with the tile's number.
- **Order.** Drag a title bar out of its place, and each tile shows a drop area with its number.
  Release on another tile to swap the two tiles. Release on your own tile or on the bar to cancel.
  Make main moves a tile to place 1. New tiles go last. Helm keeps the order for each space.
- **Focus.** The focused tile has an accent line under its title.
- **Restart.** An agent tile has a restart button. `herdr-helm restart --terminal <address>` stops
  the agent and starts it again in the same session and the same terminal. Use it after an update
  of the agent. When the agent is busy, the first click asks again.

## Top bar

From left to right:

- the grid button, which opens the spaces window,
- the actions of the shown space as buttons,
- the title, project, PR state, and agents of the shown space,
- how many other spaces need you,
- CPU and memory, agent usage, GitHub, Changes, Files, and the ⋯ menu.

The parts on the right show only when a connector serves them, see
[connectors.md](connectors.md).

## Spaces window

- The spaces in groups per project are on the left, with a + for each project and for Other.
- All the information of the selected space is on the right, with its action buttons: the
  summary and the pieces of work, the agents, the PRs, the checkouts, and the agent sessions.
- Arrows and `tab` move. `enter` shows the space, and so does `space` while the filter is empty.
- Typing filters, and `cmd-backspace` clears the filter. `alt-space` or `esc` closes the window.

## alt-tab

`alt-tab` opens the same window, with the last space before the shown one selected.

- A quick `alt-tab`, where you release `alt` within 300 ms, goes back to that space without the
  window.
- Hold `alt` longer, or press `tab` again, and the window shows. It stays open until `enter` or
  `esc`.

## New space

`cmd-n` opens the New space dialog.

- **A project space.** Pick a project, write a title, and pick the repos. Helm makes the name and
  the branch from the title. The first tab is an agent or a shell. The agent gets a first message
  only when you write one.
- **A plain space.** Under Other, pick a folder. The dialog lists the folders where agent sessions
  ran, newest first, and you can choose any other folder.

**Complete** removes a project space when all its work is pushed. It first lists what blocks it.
**Delete** removes the space and the local branches that Helm made. It first lists what is lost.
Only Herdr can run these two. `herdr-helm` refuses them inside a terminal, so an agent does not end
its own space by mistake.

## Space titles and Smart rename

- A space keeps the title that you give it. Helm links a new PR of its branches to the space, but
  the PR does not rename the space.
- **Smart rename** (`cmd-shift-f2`, or the sparkles button) asks an agent to read
  all the facts of the space and to name it. See
  [configuration.md](configuration.md#agent_settings-claude-code).
- The title goes to the space record, to Herdr, and to `CLAUDE.md` when the goal only repeated the
  old title. The summary and the pieces of work go to the record.
- Smart rename works on a plain space too, from the sessions in its terminals.
- `herdr-helm rename --title "..."` gives your own title. A rename in Herdr also counts as yours.

## GitHub popup

- **Top part:** the PRs of the branches of the shown space, with the numbers of the tiles that
  work on each branch. A click on a number moves the keyboard to that tile.
- **Bottom part:** the open PRs of the org, in sections by what they need from you.
- **Review** makes a space on the PR's branch, or uses the space that has it, and starts an agent
  that reviews the PR.
- **Fix** does the same for your own PR that has problems, such as failed checks or open threads.
- The agent does not push and does not post until you say so.
- **Add to queue** asks again on the first click, because other people see it. It uses
  `gh pr merge --auto`. On a repo without a merge queue, it sets auto-merge, or refuses when the
  PR can merge now.

## Changes and Files

- **Changes** shows the changed files and the unpushed commits of each repo of the space. A click
  on a file opens its diff against HEAD in the editor. **Open all** opens a new editor window with
  a diff for each file.
- Undo a file with a second click on its undo button.
- One commit message commits every ticked repo. The ↑ button of a repo pushes it.
- **Files** is a file browser for the folder of the focused tile. A click opens the file in the
  editor.

## Agent sessions after a restart

Every 30 seconds, `herdr-helm sessions save` records which agent session runs in which terminal.
herdr-core brings kept terminals back as shells after a shutdown. When Herdr starts,
`herdr-helm sessions resume` types the agent's resume command into each of them. A session that
you ended yourself does not come back.

## Opening links from a terminal

On macOS, Herdr gives a clicked link to the system. To open the file links that agents print in
your editor, run `tools/herdr-default-editor [bundle id] [extension...]` once. It makes the editor
(default: VS Code) the macOS default app for code and text files.
