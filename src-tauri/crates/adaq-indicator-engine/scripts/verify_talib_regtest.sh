#!/bin/sh
set -eu

crate_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/adaq-talib-regtest.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT
tar -xzf "$crate_dir/vendor/ta-lib-0.7.1.tar.gz" -C "$work_dir"
cmake -S "$work_dir/ta-lib-0.7.1" -B "$work_dir/build" -DBUILD_DEV_TOOLS=ON -DCMAKE_BUILD_TYPE=Release
cmake --build "$work_dir/build" --target test
