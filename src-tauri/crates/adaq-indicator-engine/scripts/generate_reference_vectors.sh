#!/bin/sh
set -eu

crate_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
archive="$crate_dir/vendor/ta-lib-0.7.1.tar.gz"
output="$crate_dir/reference_vectors.json"
check=false
if [ "${1:-}" = "--check" ]; then check=true; fi
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/adaq-talib-vectors.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT
tar -xzf "$archive" -C "$work_dir"
source_dir="$work_dir/ta-lib-0.7.1"
cmake -S "$source_dir" -B "$work_dir/build" -DBUILD_DEV_TOOLS=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build "$work_dir/build" --target ta-lib-static
library=$(find "$work_dir/build" -name 'libta-lib.a' -o -name 'ta-lib-static.lib' | head -n 1)
cc -std=c11 -O2 -I"$source_dir/include" "$crate_dir/scripts/generate_reference_vectors.c" "$library" -lm -o "$work_dir/generate-reference-vectors"
generated="$work_dir/reference_vectors.json"
"$work_dir/generate-reference-vectors" > "$generated"
test "$(jq '.indicators | length' "$generated")" = 160
test "$(jq '[.indicators[].outputs[]] | length' "$generated")" = 179
if "$check"; then cmp -s "$generated" "$output"; else cp "$generated" "$output"; fi
