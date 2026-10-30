#!/usr/bin/env bash
# Cross-compiles release binaries into dist/ (no cgo, so any host can build every target).
# Usage: scripts/build.sh [version] [os/arch ...]
set -euo pipefail
cd "$(dirname "$0")/.."
version="${1:-dev}"
shift || true
targets=("$@")
if [ ${#targets[@]} -eq 0 ]; then
  targets=(darwin/arm64 linux/arm64 darwin/amd64 linux/amd64 windows/amd64)
fi
mkdir -p dist
for t in "${targets[@]}"; do
  os="${t%/*}" arch="${t#*/}" ext=""
  [ "$os" = windows ] && ext=".exe"
  out="dist/cloute-bridge-${os}-${arch}${ext}"
  CGO_ENABLED=0 GOOS="$os" GOARCH="$arch" \
    go build -trimpath -ldflags "-s -w -X main.version=${version}" -o "$out" .
  echo "built $out"
done
# sha256sum on Linux, shasum on macOS.
(cd dist && { sha256sum cloute-bridge-* 2>/dev/null || shasum -a 256 cloute-bridge-*; } > SHA256SUMS)
