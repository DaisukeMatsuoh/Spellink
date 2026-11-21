#!/usr/bin/env bash
# Builds the plugin package WITHOUT this machine's bridge baked in, and copies it to the path given
# (default build/outputs/Spellink-public.snplg). The normal build bakes src/bridges.local.json (a
# bridge address and its token) into the bundle; a package made for other people must not have it.
# The file is put back afterwards, even if the build fails.
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-build/outputs/Spellink-public.snplg}"
local_cfg=src/bridges.local.json
keep=""
if [ -f "$local_cfg" ]; then
  keep="$(mktemp)"
  cp -p "$local_cfg" "$keep"
fi
restore() {
  if [ -n "$keep" ]; then mv -f "$keep" "$local_cfg"; else rm -f "$local_cfg"; fi
}
trap restore EXIT
printf '{"bridges": []}\n' > "$local_cfg"
bash buildPlugin.sh
mkdir -p "$(dirname "$out")"
cp build/outputs/Spellink.snplg "$out"
# the result is checked, not assumed: a zip, with nothing of the baked-in address in it
test -s "$out"
if unzip -p "$out" Spellink.bundle | grep -q 'bridges.local\|"token"[[:space:]]*:[[:space:]]*"[A-Za-z0-9_-]\{20,\}"'; then
  echo "the package seems to carry a bridge token; not using it" >&2
  rm -f "$out"
  exit 1
fi
echo "clean package: $out"
