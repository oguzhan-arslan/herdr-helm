#!/bin/sh
# Installs or updates Helm in Herdr. You can run it again at any time: it keeps your settings,
# your own keys, and your spaces.
#
# Without a clone (the newest release):
#   curl -fsSL https://raw.githubusercontent.com/oguzhan-arslan/herdr-helm/main/install.sh | sh
#
# In a clone:
#   ./install.sh               install, or update after `git pull`
#   ./install.sh --dry-run     show what would change in Herdr, change nothing there
#   ./install.sh --uninstall   remove Helm from Herdr (projects.json and the spaces stay)
#
# The work is done by the herdr-helm program, which has the mods, the layout, the keys, and
# the templates inside it. This script gets the program first:
#   - in a clone with cargo on the PATH, it builds the program from the clone,
#   - else it downloads a release build for this machine (with gh when it can, so that a
#     private repo works too), and checks its SHA-256 sum. The sum comes from the same
#     release, so it finds a damaged download, not a changed release.
# In a clone, a download needs program code that is the same as the release tag of the
# version in Cargo.toml. A clone with newer code needs cargo, so that program and mods match.
# --uninstall uses the installed program when there is one, and gets nothing.
# HELM_BUILD=download skips cargo. HELM_BUILD=cargo needs cargo.
# HELM_REPO=<owner>/<name> installs from a fork.
set -eu

source_repo=${HELM_REPO:-oguzhan-arslan/herdr-helm}
host=github.com
mode=${HELM_BUILD:-auto}

say() { printf '%s\n' "$*" >&2; }
fail() { say "install.sh: $*"; exit 1; }

# A clone: this script is a file next to Cargo.toml and mods/. Piped into sh, $0 is no file.
repo=""
if [ -f "$0" ]; then
  dir=$(cd "$(dirname "$0")" && pwd)
  if [ -f "$dir/Cargo.toml" ] && [ -d "$dir/mods" ]; then repo=$dir; fi
fi

# The Rust target name of this machine, as the release files use it.
target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$arch" in
    arm64 | aarch64) arch=aarch64 ;;
    x86_64 | amd64) arch=x86_64 ;;
    *) fail "no release build for the CPU type $arch. Install Rust (https://rustup.rs), clone the repo, and run ./install.sh." ;;
  esac
  case "$os" in
    Darwin) echo "$arch-apple-darwin" ;;
    Linux) echo "$arch-unknown-linux-musl" ;;
    *) fail "no release build for $os. Install Rust (https://rustup.rs), clone the repo, and run ./install.sh." ;;
  esac
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d ' ' -f 1
  else shasum -a 256 "$1" | cut -d ' ' -f 1; fi
}

# The tag of the newest release of the source repo.
latest_tag() {
  if command -v gh >/dev/null 2>&1 && t=$(gh api --hostname "$host" "repos/$source_repo/releases/latest" --jq .tag_name 2>/dev/null); then
    echo "$t"
    return
  fi
  curl -fsSL --max-time 20 "https://api.$host/repos/$source_repo/releases/latest" | sed -n 's/^ *"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1 | grep . ||
    fail "cannot find a release of $source_repo. For a private repo, install gh and run 'gh auth login'."
}

# Downloads the program of release $1 into $2, and checks it.
download() {
  tag=$1
  out=$2
  name="herdr-helm-$(target).tar.gz"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  say "Downloading $name of $tag from $source_repo..."
  if command -v gh >/dev/null 2>&1 && gh release download "$tag" -R "$host/$source_repo" -p "$name" -p "$name.sha256" -D "$tmp" 2>/dev/null; then :
  else
    base="https://$host/$source_repo/releases/download/$tag"
    { curl -fsSL "$base/$name" -o "$tmp/$name" && curl -fsSL "$base/$name.sha256" -o "$tmp/$name.sha256"; } ||
      fail "cannot download $name of $tag. For a private repo, install gh and run 'gh auth login'."
  fi
  want=$(cut -d ' ' -f 1 < "$tmp/$name.sha256")
  [ "$(sha256 "$tmp/$name")" = "$want" ] || fail "the SHA-256 sum of $name is wrong. Nothing changed."
  tar -xzf "$tmp/$name" -C "$tmp"
  mkdir -p "$(dirname "$out")"
  mv "$tmp/herdr-helm" "$out"
  chmod +x "$out"
}

# --uninstall can be anywhere in the arguments. The other arguments go to the program.
uninstall=false
rest=""
for a in "$@"; do
  if [ "$a" = "--uninstall" ]; then uninstall=true; else rest="$rest $a"; fi
done
installed="${HERDR_APP_HOME:-$HOME/.config/herdr-app}/helm/bin/herdr-helm"
if $uninstall && [ -x "$installed" ]; then
  # shellcheck disable=SC2086 # the flags are single words
  exec "$installed" uninstall $rest
fi
action=install
$uninstall && action=uninstall

if [ -z "$repo" ]; then
  # No clone: the newest release. Its program installs the files inside it.
  [ "$mode" = cargo ] && fail "HELM_BUILD=cargo needs a clone of the repo."
  tag=$(latest_tag) || exit 1
  [ -n "$tag" ] || fail "cannot find a release of $source_repo."
  bin="$(mktemp -d)/herdr-helm"
  download "$tag" "$bin"
  # With set -e, a failed program would end the script here: keep its status instead.
  status=0
  # shellcheck disable=SC2086
  "$bin" $action $rest || status=$?
  rm -rf "$(dirname "$bin")"
  exit $status
fi

bin="$repo/target/release/herdr-helm"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -n 1)

build() {
  say "Building herdr-helm $version with cargo..."
  (cd "$repo" && cargo build --release --locked --quiet)
}

download_for_clone() {
  tag="v$version"
  # The release program must match the mods of this clone.
  if git -C "$repo" rev-parse -q --verify "refs/tags/$tag" >/dev/null 2>&1; then
    git -C "$repo" diff --quiet "$tag" -- src Cargo.toml Cargo.lock ||
      fail "this clone has program code that is newer than release $tag. Install Rust (https://rustup.rs) to build it, or check out $tag."
  else
    say "note: no tag $tag in this clone, so the release cannot be compared with it."
  fi
  download "$tag" "$bin"
}

case "$mode" in
  cargo) command -v cargo >/dev/null 2>&1 || fail "HELM_BUILD=cargo, but cargo is not on the PATH."; build ;;
  download) download_for_clone ;;
  auto) if command -v cargo >/dev/null 2>&1; then build; else download_for_clone; fi ;;
  *) fail "HELM_BUILD must be auto, cargo, or download." ;;
esac

# shellcheck disable=SC2086
exec "$bin" $action --from "$repo" $rest
