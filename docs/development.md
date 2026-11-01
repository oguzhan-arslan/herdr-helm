# Development

## Set up

1. Install Rust with [rustup](https://rustup.rs). Helm needs the `rust-version` of `Cargo.toml`
   or newer.
2. Clone the repo and run `./install.sh`. With cargo on the PATH, it builds the program from your
   clone and installs it.
3. Put `~/.local/bin` on your PATH, so that you and your agents can run `herdr-helm`.

`install.sh` reads `HELM_BUILD`:

| Value | What it does |
| --- | --- |
| `auto` (default) | Builds with cargo when cargo is on the PATH, else downloads the release. |
| `cargo` | Builds with cargo, and fails without it. |
| `download` | Downloads the release of the version in `Cargo.toml`, and checks its SHA-256 sum. |

A download works only when the program code of the clone (`src/`, `Cargo.toml`, `Cargo.lock`) is
the same as the release tag. Else the program and the mods could differ, so `install.sh` asks for
cargo. The SHA-256 file comes from the same release, so the check finds a damaged download, not a
changed release.

## The work cycle

1. Change the files in the repo. Never change the installed copy in the Herdr home, because the
   next install replaces it.
2. Run `./install.sh`. Herdr reloads the mods at once.
3. Run the tests: `cargo test`.
4. Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `shellcheck install.sh`. The
   CI fails on a format difference or a warning.

`./install.sh --dry-run` shows what an install changes.

## Tests

- **Unit tests** are next to the code, in `#[cfg(test)]` modules. Some make real git repos in a
  temp folder.
- **`tests/mods.rs`** checks the mods without Herdr: the form of each `mod.json`, the overlay
  sizes against `OVERLAYS`, the shared imports, the icon paths, the key commands, and old names
  that must not come back.
- A test that sets an env var, such as `HERDR_PROJECTS`, holds `util::env_lock()`, because the
  tests run in parallel threads.

The mods themselves have no automatic tests. Check a mod change in Herdr.

## Rules for the mods

- Shared code goes into `mods/_shared/`. Do not copy code between mods.
- Call `herdr-helm` through `helm()` or `helmSlow()` of `helm.js`, never by a path of your own.
- Open an overlay through `overlay()`, and put its size in `OVERLAYS`. When the mod's `mod.json`
  has an `overlay`, it must have the same size.
- Every mod has a `mod.json` with `"place": "none"`. Without it, Herdr puts the mod into the shown
  layout.
- Hide a part that needs a connector with `can(kind, capability)`.
- A write to a shared value makes every mod render again. Write only a changed value, and never
  write in `render`.
- Herdr stops a render that takes too long. Keep the work of each render small.
- Icons: use a mod's own SVG files, or the icons of Herdr's `workspace-chrome` mod. The installer
  warns when a Herdr mod that Helm uses is missing.

## Add a connector

**A built-in connector:**

1. Add a file in `src/connectors/<kind>/`, for example `gitlab.rs`.
2. Implement the trait of the kind. It is in `src/connectors/<kind>/mod.rs`.
3. Add its name to `current()` in the same file.
4. Name only the capabilities that it supports.
5. Add unit tests, and a section in the doc of the kind.

**An external connector:** write a program that follows the JSON contract in
[connectors.md](connectors.md). No change to Helm is necessary.

## Release

1. Set the new version in `Cargo.toml`, and run `cargo build` to update `Cargo.lock`.
2. Commit, then tag the commit `v<version>`, for example `v0.2.0`, and push the tag.
3. The CI checks the tag, builds the program for macOS (arm64 and x86_64) and Linux (x86_64 and
   arm64, static musl builds), and makes a GitHub release with
   `herdr-helm-<target>.tar.gz` and its `.sha256` file. A second run of the same tag replaces
   the files.

## Updates

`herdr-helm update --check` asks the GitHub releases of the source repo for the newest version,
at most every 6 hours, and keeps the answer in `state/update.json`. helm-jobs runs it every hour
and puts the answer in the shared value `helm.update`. `herdr-helm update` downloads the release
program for this machine, checks its SHA-256 sum, and runs its `install`. The source repo is
`repository` in `Cargo.toml`. `HELM_REPO=<owner>/<name>` sets a fork. A private repo needs `gh`,
signed in.

The program has the mods, the layout, the keys, and the templates inside it (`build.rs` and
`src/assets.rs`). So a release is one file for each platform, and an install needs no clone.

## CI

`.github/workflows/ci.yml` runs on each push to `main`, on each pull request, and on each tag:

- `lint`: `cargo fmt --check`, clippy with `-D warnings`, and shellcheck.
- `test`: `cargo test` on Linux and on macOS.
- `msrv`: a build with the `rust-version` of `Cargo.toml`.
- `build` and `release`: only on a `v*` tag.

The actions are pinned to commit SHAs, because the release job can write to the repo. To update
an action, change the SHA and the version comment together.

## Platforms

Helm is tested on macOS. The Linux system connector and the Linux builds exist, but nobody tested
Helm on Linux yet. The `usage` feature of claude-code reads the macOS keychain, so it works on
macOS only.
