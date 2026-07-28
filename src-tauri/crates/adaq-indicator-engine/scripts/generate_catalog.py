#!/usr/bin/env python3
"""Generate the frozen ADAQ catalog from the vendored TA-Lib XML."""

import argparse
import hashlib
import json
import re
import tarfile
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ARCHIVE = ROOT / "vendor" / "ta-lib-0.7.1.tar.gz"
OUTPUT = ROOT / "catalog.json"
XML_PATH = "ta-lib-0.7.1/ta_func_api.xml"

OUTPUT_IDS = {
    "outRealUpperBand": "upper-band", "outRealMiddleBand": "middle-band",
    "outRealLowerBand": "lower-band", "outMACD": "macd", "outMACDSignal": "signal",
    "outMACDHist": "histogram", "outSlowK": "slow-k", "outSlowD": "slow-d",
    "outFastK": "fast-k", "outFastD": "fast-d", "outReal": "value",
    "outInteger": "value",
}
MA_TYPES = ["sma", "ema", "wma", "dema", "tema", "trima", "kama", "mama", "t3"]

def kebab(value):
    return "-".join(re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", value).replace("_", " ").split()).lower()

def child(element, name, default=""):
    value = element.findtext(name)
    return value if value is not None else default

def optional(argument):
    parameter_type = child(argument, "Type")
    range_element = argument.find("Range")
    item = {
        "id": kebab(child(argument, "Name")),
        "rawName": child(argument, "Name"),
        "type": parameter_type,
        "default": child(argument, "DefaultValue"),
        "minimum": child(range_element, "Minimum") if range_element is not None else "",
        "maximum": child(range_element, "Maximum") if range_element is not None else "",
    }
    if parameter_type == "MA Type":
        item["enumValues"] = [{"id": name, "value": index} for index, name in enumerate(MA_TYPES)]
    return item

def required_inputs(arguments):
    generic_index = 0
    inputs = []
    for argument in arguments:
        input_type = child(argument, "Type")
        if input_type in {"Open", "High", "Low", "Close"}:
            item = {"id": input_type.lower(), "name": child(argument, "Name"), "type": input_type, "role": "fixed-market"}
        elif input_type == "Volume":
            item = {"id": "volume", "name": child(argument, "Name"), "type": input_type, "role": "explicit-volume", "allowedFields": ["base-volume", "quote-volume"]}
        else:
            item = {"id": f"real-{generic_index}", "name": child(argument, "Name"), "type": input_type, "role": "generic-ohcl-real", "allowedFields": ["open", "high", "low", "close"]}
            generic_index += 1
        inputs.append(item)
    return inputs

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    with tarfile.open(ARCHIVE, "r:gz") as archive:
        xml = archive.extractfile(XML_PATH).read()
    root = ET.fromstring(xml)
    indicators = []
    for function in root.findall("FinancialFunction"):
        raw_name = child(function, "Abbreviation")
        if raw_name == "MAVP":
            continue
        outputs = []
        for output in function.findall("./OutputArguments/OutputArgument"):
            raw_output = child(output, "Name")
            outputs.append({
                "id": OUTPUT_IDS.get(raw_output, kebab(raw_output.removeprefix("out"))),
                "rawName": raw_output,
                "type": child(output, "Type"),
            })
        indicators.append({
            "id": raw_name.lower().replace("_", "-"),
            "rawName": raw_name,
            "group": child(function, "GroupId"),
            "unstablePeriod": "Unstable Period" in [flag.text for flag in function.findall("./Flags/Flag")],
            "inputs": required_inputs(function.findall("./RequiredInputArguments/RequiredInputArgument")),
            "parameters": [optional(argument) for argument in function.findall("./OptionalInputArguments/OptionalInputArgument")],
            "outputs": outputs,
        })
    assert len(indicators) == 160, len(indicators)
    assert sum(len(item["outputs"]) for item in indicators) == 179
    assert len({item["id"] for item in indicators}) == 160
    for item in indicators:
        assert len({output["id"] for output in item["outputs"]}) == len(item["outputs"]), item["rawName"]
    document = {
        "version": "adaq-indicator-catalog@1.0.0",
        "source": {"archiveSha256": hashlib.sha256(ARCHIVE.read_bytes()).hexdigest(), "xmlSha256": hashlib.sha256(xml).hexdigest()},
        "indicators": indicators,
    }
    rendered = json.dumps(document, indent=2, sort_keys=True) + "\n"
    if args.check:
        assert OUTPUT.read_text() == rendered, "catalog.json is stale; run scripts/generate_catalog.py"
    else:
        OUTPUT.write_text(rendered)

if __name__ == "__main__":
    main()
