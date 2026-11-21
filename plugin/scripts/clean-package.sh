#!/usr/bin/env bash
# Builds the plugin package and copies it to the path given (default build/outputs/Spellink-public.snplg).
# A package carries no bridge address or token (the device gets them by pairing, ADR 0016), so any
# build is fit to hand out; this script only gives the result a fixed name and checks it.
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-build/outputs/Spellink-public.snplg}"
bash buildPlugin.sh
mkdir -p "$(dirname "$out")"
cp build/outputs/Spellink.snplg "$out"
# the result is checked, not assumed: a non-empty zip with the bundle in it
test -s "$out"
unzip -l "$out" | grep -q 'Spellink.bundle'
echo "package: $out"
