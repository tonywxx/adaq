#!/usr/bin/env python3
"""Generate public Catalog and Manifest references from their contracts."""

import argparse
import json
from pathlib import Path

CRATE = Path(__file__).resolve().parents[1]
REPO = CRATE.parents[2]
CATALOG = CRATE / "catalog.json"
CONTRACT = REPO / "src-tauri/crates/adaq-component-tooling/component-manifest.contract.json"
def catalog_reference(catalog):
    lines = ["# ADAQ Indicator Catalog", "", "Generated from `catalog.json`; do not edit.", "",
             f"{len(catalog['indicators'])} Indicators / {sum(len(item['outputs']) for item in catalog['indicators'])} outputs. Lookback is available from the host Catalog for every entry; `unstablePeriod` is the official TA-Lib flag, not a convergence claim.", ""]
    for item in catalog["indicators"]:
        lines += [f"## `{item['id']}` ({item['rawName']})", "", f"Group: {item['group']}. Official Unstable Period: `{str(item['unstablePeriod']).lower()}`.", "",
                  "### Inputs", ""]
        lines += [f"- `{value['id']}` — {value['type']}; {value['role']}." for value in item["inputs"]] or ["- None."]
        lines += ["", "### Parameters", ""]
        lines += [f"- `{value['id']}` — {value['type']}; default `{value['default']}`; range `{value['minimum']}`–`{value['maximum']}`." for value in item["parameters"]] or ["- None."]
        lines += ["", "### Outputs", ""]
        lines += [f"- `{value['id']}` — {value['type']}." for value in item["outputs"]]
        lines += [""]
    return "\n".join(lines)

def manifest_reference(contract):
    lines = ["# ADAQ Component Manifest", "", "Generated from `component-manifest.contract.json`; do not edit.", "",
             "| Field | Contract |", "| --- | --- |"]
    for name, value in contract["properties"].items():
        rule = value.get("description", "")
        if "const" in value:
            rule += f" Exact value: `{value['const']}`."
        if "enum" in value:
            rule += " Values: " + ", ".join(f"`{item}`" for item in value["enum"]) + "."
        lines.append(f"| `{name}` | {rule} |")
    return "\n".join(lines) + "\n"

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
    rendered = {
        REPO / "docs/reference/indicator-catalog.md": catalog_reference(catalog),
        REPO / "docs/reference/component-manifest.schema.json": json.dumps(contract, indent=2, sort_keys=True) + "\n",
        REPO / "docs/reference/component-manifest.md": manifest_reference(contract),
    }
    for path, text in rendered.items():
        if args.check:
            assert path.read_text(encoding="utf-8").replace("\r\n", "\n") == text, f"{path.relative_to(REPO)} is stale; run scripts/generate_references.py"
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")

if __name__ == "__main__":
    main()
