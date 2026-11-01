# Space: $space

This folder is a Herdr space in the project `$project`. It holds git worktrees
of the project repos. The `herdr-helm` script made this file. It writes the file
again when a checkout changes, so do not edit it by hand.

## Goal

$goal

## Checkouts

| Folder | Repo | Branch | Kind |
| --- | --- | --- | --- |
$checkouts

Repos you can add: $available

## Rules

- Change files only inside `$space_dir`.
- Never change files in the main checkouts under `$root`. Other work uses them.
- Each repo has its own git history. Commit, push, and open PRs per repo.
- Each repo can have its own `CLAUDE.md`. Read it before you change that repo.
- Use the `task-workspace` skill to add a repo, to work in parallel, and to talk
  to other agents in this space.
- Do not run `herdr-helm complete` or `herdr-helm delete`. Only the user ends a space.
