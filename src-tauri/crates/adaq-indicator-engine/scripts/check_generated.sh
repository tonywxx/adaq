#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
python3 "$script_dir/generate_catalog.py" --check
python3 "$script_dir/generate_references.py" --check
sh "$script_dir/generate_reference_vectors.sh" --check
