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
compiler=$(sed -n 's/^CMAKE_C_COMPILER:FILEPATH=//p' "$work_dir/build/CMakeCache.txt" | head -n 1)
generated="$work_dir/reference_vectors.json"
if [ "${OS:-}" = Windows_NT ]; then
  executable="$work_dir/generate-reference-vectors.exe"
  "$compiler" /nologo /std:c11 /O2 "/I$source_dir/include" "$crate_dir/scripts/generate_reference_vectors.c" "$library" "/Fe$executable"
else
  executable="$work_dir/generate-reference-vectors"
  "$compiler" -std=c11 -O2 -I"$source_dir/include" "$crate_dir/scripts/generate_reference_vectors.c" "$library" -lm -o "$executable"
fi
"$executable" > "$generated"
test "$(jq '.indicators | length' "$generated")" = 160
test "$(jq '[.indicators[].outputs[]] | length' "$generated")" = 179
if "$check"; then
  python_bin=${PYTHON:-python3}
  command -v "$python_bin" >/dev/null 2>&1 || python_bin=python
  "$python_bin" - "$generated" "$output" <<'PY'
import json, math, sys

def compare(actual, expected, path="$"):
    if isinstance(actual, dict) and isinstance(expected, dict):
        assert actual.keys() == expected.keys(), path
        for key in actual: compare(actual[key], expected[key], f"{path}.{key}")
    elif isinstance(actual, list) and isinstance(expected, list):
        assert len(actual) == len(expected), path
        for index, (left, right) in enumerate(zip(actual, expected)):
            compare(left, right, f"{path}[{index}]")
    elif isinstance(actual, int) and isinstance(expected, int):
        assert actual == expected, path
    elif isinstance(actual, (int, float)) and isinstance(expected, (int, float)):
        assert math.isfinite(actual) and math.isfinite(expected), path
        assert abs(actual - expected) <= 1e-12 or abs(actual - expected) / max(abs(actual), abs(expected)) <= 1e-12, path
    else:
        assert actual == expected, path

with open(sys.argv[1]) as actual, open(sys.argv[2]) as expected:
    compare(json.load(actual), json.load(expected))
PY
else
  cp "$generated" "$output"
fi
