#!/usr/bin/env bash
set -euo pipefail

threshold="${1:-1000}"

allowed='
crates/headroom-core/src/pw/registry.rs
crates/headroom-gui/src/app.rs
crates/headroom-core/src/profile_store.rs
crates/headroom-cli/src/tui.rs
crates/headroom-core/src/ipc/ops.rs
crates/headroom-dsp/src/limiter.rs
crates/headroom-core/src/app_level.rs
'

is_allowed() {
  local file="$1"
  grep -qxF "$file" <<<"$allowed"
}

status=0
while IFS= read -r -d '' file; do
  lines="$(wc -l <"$file")"
  if (( lines <= threshold )); then
    continue
  fi
  if is_allowed "$file"; then
    printf 'warn: %s has %s lines allowed over %s\n' "$file" "$lines" "$threshold"
  else
    printf 'error: %s has %s lines over %s\n' "$file" "$lines" "$threshold" >&2
    status=1
  fi
done < <(find crates -type f -name '*.rs' -print0 | sort -z)

exit "$status"
