#!/usr/bin/env bash
# Builds the bridge with a clean plugin package inside (so that `device add` needs no setup).
# Usage: scripts/build-with-plugin.sh [cargo build args]   (default: --release --locked)
# Needs the plugin's node_modules (`npm ci` in plugin/). The package comes from
# plugin/scripts/clean-package.sh; see bridge/build.rs for why it is not found by a default path.
set -euo pipefail
cd "$(dirname "$0")/.."
pkg="$PWD/target/Spellink-public.snplg"
mkdir -p target
(cd ../plugin && bash scripts/clean-package.sh "$pkg")
[ $# -eq 0 ] && set -- --release --locked
SPELLINK_BASE_PACKAGE="$pkg" cargo build "$@"
