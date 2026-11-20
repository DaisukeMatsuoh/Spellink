#!/usr/bin/env bash
# Measures the Go bridge and this Rust prototype the same way (see README.md):
#   1. resident memory with the glyph data loaded and nothing happening (the bridge waits most of the time)
#   2. peak memory and time while composing the same answer many times
#   3. binary size
# Usage: ./measure.sh GO_BINARY [GLYPHS] [ITERATIONS]
set -euo pipefail
cd "$(dirname "$0")"
GO=${1:?the Go bridge binary}
G=${2:-$HOME/.config/spellink-bridge/glyphs.bin}
N=${3:-200}
RS=target/release/spellink-bridge-rs
L='{"x":84,"y":300,"width":1200,"pageHeight":1872,"bottomMargin":84,"charSize":47,"maxPages":3,"continueTop":84}'
TEXT=$(mktemp)
printf '%s' "富士山は日本で一番高い山で、標高は3776メートルです。静岡県と山梨県にまたがり、よく晴れた日には東京からも見えます。古くから信仰の対象とされ、多くの芸術作品の題材にもなってきました。2013年には世界文化遺産に登録されています。夏の登山シーズンには、多くの人が山頂を目指します。Mount Fuji is an active stratovolcano that last erupted in 1707." > "$TEXT"

kb() { awk -v k="$2" '$1==k":" {print $2}' "/proc/$1/status"; }
idle() { # $@: command; prints "rss_kb peak_kb"
  "$@" >/dev/null 2>&1 & local pid=$!
  sleep 2
  echo "$(kb $pid VmRSS) $(kb $pid VmHWM)"
  kill $pid 2>/dev/null; wait $pid 2>/dev/null || true
}
peak() { # runs to the end, sampling the peak; prints "peak_kb"
  "$@" >/tmp/measure.out 2>&1 & local pid=$! p=0 v
  while kill -0 $pid 2>/dev/null; do v=$(kb $pid VmHWM 2>/dev/null || echo 0); [ -n "$v" ] && [ "$v" -gt "$p" ] && p=$v; sleep 0.02; done
  wait $pid; echo "$p"
}

printf '%-6s %10s %10s %12s %12s %10s\n' impl idleRSS idlePeak benchPeak msPerAnswer binary
for impl in go rust; do
  if [ $impl = go ]; then B=$GO; else B=$RS; fi
  read -r rss hwm < <(idle "$B" idle --glyphs "$G" --seconds 5)
  bp=$(peak "$B" bench --glyphs "$G" --layout "$L" --text "$TEXT" --iterations "$N")
  ms=$(sed -E 's/.*"msPerAnswer":([0-9.]+).*/\1/' /tmp/measure.out)
  size=$(stat -c %s "$B")
  printf '%-6s %8s KB %8s KB %10s KB %12s %8s KB\n' $impl "$rss" "$hwm" "$bp" "$ms" $((size/1024))
done
rm -f "$TEXT" /tmp/measure.out
