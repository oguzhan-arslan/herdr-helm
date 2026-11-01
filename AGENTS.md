# AGENTS.md

This file is the start point for a coding agent that works on Helm. Helm is a layout for
[Herdr](https://herdr.dev): JavaScript mods, a layout, keys, and the Rust program `herdr-helm`.

## Read first

- [docs/architecture.md](docs/architecture.md): the parts, the mods, and the shared values.
- [docs/development.md](docs/development.md): the work cycle, the tests, and the rules for mods.
- For a connector: [docs/connectors.md](docs/connectors.md) and the page of its kind.
- For the core: [docs/core.md](docs/core.md).

## Commands

```sh
cargo build                                   # build
cargo test                                    # unit tests and tests/mods.rs
cargo clippy --all-targets -- -D warnings     # the CI fails on a warning
./install.sh --dry-run                        # what an install would change
./install.sh                                  # install into the Herdr home
```

## Rules

- Change the files in this repo. Never change the installed copy in the Herdr home
  (`$HERDR_APP_HOME`, default `~/.config/herdr-app`), because the next install replaces it.
- Every outside service goes through a connector. The core and the mods never call `gh`,
  `claude`, `code`, or similar tools directly. Herdr and `hc` are the host, not a connector.
  One exception: `herdr-helm update` and `install.sh` call `gh` or `curl` directly, because they
  get Helm itself from its own GitHub releases, whatever code host you use.
- A connector names only the capabilities that it supports. The mods hide what is missing with
  `can(kind, capability)` from `mods/_shared/helm.js`.
- Shared mod code goes into `mods/_shared/`. The installer copies it into each mod.
- Overlay sizes live in `OVERLAYS` in `mods/_shared/helm.js`. A `mod.json` overlay repeats its
  entry, and `tests/mods.rs` checks that they match.
- Keep `--json` output stable. The mods read it, and so do agents in a space.
- `complete` and `delete` must refuse to run inside a terminal (`HC_TERMINAL_ID`), so that an
  agent does not end its own space by mistake. This check is a guard, not a security limit.
- Write the docs, comments, and commit messages in ASD-STE100 Simplified Technical English.
  Exception: `README.md` is the public front page. Write it in plain, natural English.
- Add or change a test with each change in behavior.

## Where things are

| Path | What it is |
| --- | --- |
| `src/` | The `herdr-helm` program. |
| `mods/` | The Herdr mods. `mods/_shared/` is not a mod. |
| `layouts/helm.json` | The layout. |
| `keys/helm.json` | The default keys. |
| `templates/` | The `CLAUDE.md` and the built-in skills of a space. |
| `install.sh` | Gets the program (cargo or a release), then runs `herdr-helm install`. |
| `.github/workflows/ci.yml` | Tests on each push, and the release builds on a `v*` tag. |
| `tools/` | Optional helpers, for example `herdr-default-editor` on macOS. |
