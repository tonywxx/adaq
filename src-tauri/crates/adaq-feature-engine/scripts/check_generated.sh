#!/bin/sh
# Verifies the committed Feature reference vectors regenerate without diff.
# Mirrors the indicator engine's generated-artifact gate: the fixture inputs
# live in Rust code, and the committed JSON must match an exact regeneration
# on any supported platform.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
repo_root=$(CDPATH= cd -- "$crate_dir/../../.." && pwd)
relative_vectors="src-tauri/crates/adaq-feature-engine/fixtures/feature-reference-vectors.json"

ADAQ_FEATURE_REGENERATE=1 cargo test \
    --manifest-path "$crate_dir/Cargo.toml" \
    --test reference_fixtures regenerate_reference_vectors -- --ignored

if command -v git >/dev/null 2>&1; then
    cd "$repo_root"
    if ! git diff --exit-code -- "$relative_vectors"; then
        echo "Feature reference vectors drifted; commit the regenerated file." >&2
        exit 1
    fi
fi
