#!/bin/sh
# Regenerates the committed Factor journey vectors and Metric Catalog reference.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
repo_root=$(CDPATH= cd -- "$crate_dir/../../.." && pwd)

ADAQ_FACTOR_REGENERATE=1 cargo test \
    --manifest-path "$crate_dir/Cargo.toml" \
    --test reference_fixtures committed_reference_vectors_match_three_market_journeys \
    -- --exact

cargo test \
    --manifest-path "$crate_dir/Cargo.toml" \
    --test metric_golden regenerate_factor_metric_catalog \
    -- --ignored --exact

cd "$repo_root"
git diff --exit-code -- \
    src-tauri/crates/adaq-factor-research/fixtures/factor-reference-vectors.json \
    src-tauri/crates/adaq-factor-research/fixtures/factor-metric-catalog.json
