#!/usr/bin/env bash
# Builds a release binary for each target into dist/ (ADR 0018). Run it on a machine of that
# system (or with the target's C toolchain installed): the TLS library has a little C in it.
# Usage: scripts/build.sh [version] [target ...]   (no target: this machine's own)
set -euo pipefail
cd "$(dirname "$0")/.."
version="${1:-$(cargo pkgid | sed 's/.*[#@]//')}"
shift || true
targets=("$@")
[ ${#targets[@]} -eq 0 ] && targets=("$(rustc -vV | sed -n 's/^host: //p')")
mkdir -p dist
for t in "${targets[@]}"; do
  cargo build --release --locked --target "$t"
  ext=""; case "$t" in *windows*) ext=".exe";; esac
  name="spellink-bridge-${t}${ext}"
  cp "target/${t}/release/spellink-bridge${ext}" "dist/${name}"
  echo "built dist/${name}"
done
(cd dist && { sha256sum spellink-bridge-* 2>/dev/null || shasum -a 256 spellink-bridge-*; } > SHA256SUMS)
