#!/usr/bin/env python3
"""Convert external Kronos OHLCV forecasts into an AdaQ .adaq-signals archive."""

from __future__ import annotations

import argparse
import calendar
import hashlib
import importlib.metadata
import json
import math
import pathlib
import platform
import random
import sqlite3
import sys
import zipfile
from datetime import datetime, timezone


ADAPTER_VERSION = "1.0.0"
KRONOS_SOURCE_REVISION = "67b630e67f6a18c9e9be918d9b4337c960db1e9a"
MODEL_REVISION = "901c26c1332695a2a8f243eb2f37243a37bea320"
TOKENIZER_REVISION = "0e0117387f39004a9016484a186a908917e22426"
FIXED_INTERVAL_MS = {
    "1s": 1_000,
    "1m": 60_000,
    "3m": 180_000,
    "5m": 300_000,
    "15m": 900_000,
    "30m": 1_800_000,
    "1h": 3_600_000,
    "2h": 7_200_000,
    "4h": 14_400_000,
    "6h": 21_600_000,
    "12h": 43_200_000,
    "1d": 86_400_000,
    "2d": 172_800_000,
    "3d": 259_200_000,
    "5d": 432_000_000,
    "1w": 604_800_000,
}


def canonical_json(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def next_open_time(open_time_ms: int, interval: str) -> int:
    if interval in FIXED_INTERVAL_MS:
        return open_time_ms + FIXED_INTERVAL_MS[interval]
    if interval not in {"1mo", "3mo"}:
        raise ValueError(f"unsupported AdaQ Bar Interval: {interval}")
    value = datetime.fromtimestamp(open_time_ms / 1000, timezone.utc)
    months = 1 if interval == "1mo" else 3
    month_index = value.month - 1 + months
    year = value.year + month_index // 12
    month = month_index % 12 + 1
    day = min(value.day, calendar.monthrange(year, month)[1])
    return int(value.replace(year=year, month=month, day=day).timestamp() * 1000)


def transform_paths(
    snapshot: dict, generated_paths: dict[str, list[dict]], horizons: list[int], lookback: int
) -> list[dict]:
    if not horizons or any(horizon <= 0 for horizon in horizons) or len(set(horizons)) != len(horizons):
        raise ValueError("horizons must be unique positive Bar counts")
    bars = snapshot["bars"]
    if not bars:
        raise ValueError("Snapshot contains no Closed Bars")
    rows = []
    segment_length = 0
    previous_open = None
    instrument = f'{snapshot["src"]}:{snapshot["code"]}'
    for bar in bars:
        open_time = int(bar["openTimeMs"])
        if previous_open is None or open_time != next_open_time(previous_open, snapshot["interval"]):
            segment_length = 0
        segment_length += 1
        previous_open = open_time
        prediction_time = next_open_time(open_time, snapshot["interval"])
        row = {
            "instrumentId": instrument,
            "predictionTimeMs": prediction_time,
            "availableAtMs": prediction_time,
        }
        path = generated_paths.get(str(prediction_time))
        if segment_length < lookback:
            row.update(status="unavailable", values=None, unavailableReason="warmup")
        elif path is None:
            row.update(status="unavailable", values=None, unavailableReason="missing-input")
        else:
            close = float(bar["close"])
            if not math.isfinite(close) or close <= 0 or len(path) < max(horizons):
                raise ValueError(f"invalid generated path at Prediction Time {prediction_time}")
            forecast_closes = [float(path[horizon - 1]["close"]) for horizon in horizons]
            if not all(math.isfinite(value) and value > 0 for value in forecast_closes):
                raise ValueError(f"invalid generated path at Prediction Time {prediction_time}")
            values = [round(value / close - 1.0, 15) for value in forecast_closes]
            if not all(math.isfinite(value) for value in values):
                raise ValueError(f"non-finite generated path at Prediction Time {prediction_time}")
            row.update(status="present", values=values, unavailableReason=None)
        rows.append(row)
    return rows


def load_snapshot(database: pathlib.Path, user_id: str, snapshot_id: str) -> dict:
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
        result = connection.execute(
            "SELECT s.metadata_json FROM market_data_snapshots s "
            "JOIN market_data_snapshot_access a USING(snapshot_id) "
            "WHERE a.user_id = ? AND s.snapshot_id = ?",
            (user_id, snapshot_id),
        ).fetchone()
    if result is None:
        raise ValueError("Market Data Snapshot is not available to this User")
    snapshot = json.loads(result[0])
    import pyarrow.parquet as parquet

    bars = parquet.read_table(snapshot["parquetPath"]).to_pylist()
    snapshot["bars"] = [
        {
            "openTimeMs": bar["open_time_ms"],
            "open": bar["open"],
            "high": bar["high"],
            "low": bar["low"],
            "close": bar["close"],
            "baseVolume": bar["base_volume"],
            "quoteVolume": bar["quote_volume"],
        }
        for bar in bars
    ]
    return snapshot


def generate_paths(snapshot: dict, args: argparse.Namespace) -> dict[str, list[dict]]:
    import numpy
    import pandas
    import torch

    sys.path.insert(0, str(args.kronos_root))
    from model import Kronos, KronosPredictor, KronosTokenizer

    random.seed(args.seed)
    numpy.random.seed(args.seed)
    torch.manual_seed(args.seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(args.seed)
    tokenizer = KronosTokenizer.from_pretrained(str(args.tokenizer))
    model = Kronos.from_pretrained(str(args.model))
    predictor = KronosPredictor(model, tokenizer, device=args.device, max_context=512)
    bars = snapshot["bars"]
    paths = {}
    segment_start = 0
    for index, bar in enumerate(bars):
        if index and int(bar["openTimeMs"]) != next_open_time(
            int(bars[index - 1]["openTimeMs"]), snapshot["interval"]
        ):
            segment_start = index
        if index - segment_start + 1 < args.lookback:
            continue
        context = bars[index - args.lookback + 1 : index + 1]
        frame = pandas.DataFrame(
            {
                "open": [float(item["open"]) for item in context],
                "high": [float(item["high"]) for item in context],
                "low": [float(item["low"]) for item in context],
                "close": [float(item["close"]) for item in context],
                "volume": [float(item["baseVolume"]) for item in context],
                "amount": [float(item["quoteVolume"]) for item in context],
            }
        )
        x_timestamp = pandas.Series(
            pandas.to_datetime([item["openTimeMs"] for item in context], unit="ms", utc=True)
        )
        prediction_time = next_open_time(int(bar["openTimeMs"]), snapshot["interval"])
        future_times = []
        future_time = prediction_time
        for _ in range(max(args.horizons)):
            future_times.append(future_time)
            future_time = next_open_time(future_time, snapshot["interval"])
        prediction = predictor.predict(
            frame,
            x_timestamp,
            pandas.Series(pandas.to_datetime(future_times, unit="ms", utc=True)),
            pred_len=max(args.horizons),
            T=args.temperature,
            top_k=args.top_k,
            top_p=args.top_p,
            sample_count=args.sample_count,
            verbose=args.verbose,
        )
        paths[str(prediction_time)] = prediction.to_dict("records")
    return paths


def write_archive(output: pathlib.Path, snapshot: dict, rows: list[dict], args: argparse.Namespace) -> None:
    import pyarrow as arrow
    import pyarrow.parquet as parquet

    schema = arrow.schema(
        [
            arrow.field("instrument_id", arrow.string(), nullable=False),
            arrow.field("prediction_time_ms", arrow.int64(), nullable=False),
            arrow.field("available_at_ms", arrow.int64(), nullable=False),
            arrow.field("status", arrow.string(), nullable=False),
            arrow.field("forecast_json", arrow.string()),
            arrow.field("unavailable_reason", arrow.string()),
        ]
    )
    table = arrow.Table.from_pylist(
        [
            {
                "instrument_id": row["instrumentId"],
                "prediction_time_ms": row["predictionTimeMs"],
                "available_at_ms": row["availableAtMs"],
                "status": row["status"],
                "forecast_json": None if row["values"] is None else json.dumps(row["values"], separators=(",", ":")),
                "unavailable_reason": row["unavailableReason"],
            }
            for row in rows
        ],
        schema=schema,
    )
    parquet_path = output.with_suffix(".parquet.tmp")
    parquet.write_table(table, parquet_path, compression="snappy", version="2.6")
    parquet_bytes = parquet_path.read_bytes()
    parquet_path.unlink()

    adapter_hash = file_sha256(pathlib.Path(__file__))
    if args.fixture_paths:
        weight_hash = file_sha256(args.fixture_paths)
        model_configuration_hash = sha256(b"kronos-small-fixture-config")
        tokenizer_weight_hash = sha256(b"kronos-tokenizer-fixture-weights")
        tokenizer_configuration_hash = sha256(b"kronos-tokenizer-fixture-config")
        processor_hash = adapter_hash
        framework = "fixture-json@1"
    else:
        weight_hash = file_sha256(args.model / "model.safetensors")
        model_configuration_hash = file_sha256(args.model / "config.json")
        tokenizer_weight_hash = file_sha256(args.tokenizer / "model.safetensors")
        tokenizer_configuration_hash = file_sha256(args.tokenizer / "config.json")
        processor_hash = file_sha256(args.kronos_root / "model" / "kronos.py")
        versions = ", ".join(
            f"{name} {importlib.metadata.version(name)}"
            for name in ("torch", "numpy", "pandas", "pyarrow", "huggingface_hub")
        )
        framework = f"Python {platform.python_version()} on {platform.platform()}; {versions}"
    tokenizer_hash = sha256(
        canonical_json(
            {
                "revision": TOKENIZER_REVISION,
                "configurationHash": tokenizer_configuration_hash,
                "weightHash": tokenizer_weight_hash,
            }
        )
    )
    configuration = {
        "model": "NeoQuasar/Kronos-small",
        "modelRevision": MODEL_REVISION,
        "tokenizer": "NeoQuasar/Kronos-Tokenizer-base",
        "tokenizerRevision": TOKENIZER_REVISION,
        "lookbackBars": args.lookback,
        "horizons": args.horizons,
        "aggregation": "generated-close-at-horizon / origin-close - 1",
        "temperature": args.temperature,
        "topK": args.top_k,
        "topP": args.top_p,
        "sampleCount": args.sample_count,
        "seed": args.seed,
        "device": args.device,
        "frameworkRuntime": framework,
    }
    artifact_hash = sha256(
        canonical_json(
            {
                "revision": MODEL_REVISION,
                "configurationHash": model_configuration_hash,
                "weightHash": weight_hash,
            }
        )
    )
    signal_contract = {
        "outputs": [
            {
                "name": f"expected-close-return-{horizon}-bar",
                "predictionKind": {"kind": "expected-value"},
                "forecastTarget": {"kind": "builtin", "target": "future-close-return"},
                "valueScale": {"kind": "native"},
                "horizonBars": horizon,
            }
            for horizon in args.horizons
        ]
    }
    manifest = {
        "schemaVersion": 1,
        "snapshotId": snapshot["snapshotId"],
        "src": snapshot["src"],
        "code": snapshot["code"],
        "interval": snapshot["interval"],
        "parquetSha256": sha256(parquet_bytes),
        "signalContract": signal_contract,
        "producerSegments": [
            {
                "startPredictionTimeMs": rows[0]["predictionTimeMs"],
                "endPredictionTimeMs": rows[-1]["predictionTimeMs"],
                "modelArtifact": {"name": "Kronos-small", "sha256": artifact_hash, "weightSha256": weight_hash},
                "inferenceConfiguration": configuration,
                "availabilityPolicy": {"kind": "closed-bar@1"},
                "provenance": {
                    "sourceRevision": KRONOS_SOURCE_REVISION,
                    "weightHash": weight_hash,
                    "modelConfigurationHash": model_configuration_hash,
                    "tokenizerHash": tokenizer_hash,
                    "tokenizerWeightHash": tokenizer_weight_hash,
                    "tokenizerConfigurationHash": tokenizer_configuration_hash,
                    "normalizerHash": processor_hash,
                    "featureProcessorHash": processor_hash,
                    "architecture": "Kronos-small with Kronos-Tokenizer-base",
                    "frameworkRuntime": framework,
                    "adapterVersion": f"{ADAPTER_VERSION}+sha256:{adapter_hash}",
                    "licence": "MIT; inspect upstream repository and both Hugging Face model cards at the pinned revisions",
                    "source": "https://github.com/shiyu-coder/Kronos; https://huggingface.co/NeoQuasar/Kronos-small; https://huggingface.co/NeoQuasar/Kronos-Tokenizer-base",
                    "trainingWindow": "unknown: upstream publishes corpus scope but not exact record boundaries",
                    "fittingWindow": "unknown: pretrained upstream Artifact",
                    "validationWindow": "unknown: pretrained upstream Artifact",
                    "normalizationWindow": "per-prediction past-only lookback",
                    "preprocessing": "upstream KronosPredictor normalization; AdaQ Snapshot OHLC plus base/quote volume mapping",
                    "seed": args.seed,
                    "externallyGenerated": True,
                },
                "signalContract": signal_contract,
            }
        ],
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in (("manifest.json", canonical_json(manifest)), ("signals.parquet", parquet_bytes)):
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.external_attr = 0o100644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, content)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--database", type=pathlib.Path)
    source.add_argument("--snapshot-json", type=pathlib.Path)
    parser.add_argument("--user-id")
    parser.add_argument("--snapshot-id")
    parser.add_argument("--fixture-paths", type=pathlib.Path)
    parser.add_argument("--kronos-root", type=pathlib.Path)
    parser.add_argument("--model", type=pathlib.Path)
    parser.add_argument("--tokenizer", type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--lookback", type=int, default=400)
    parser.add_argument("--horizons", type=lambda value: [int(item) for item in value.split(",")], default=[1, 6, 24])
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--temperature", type=float, default=1.0)
    parser.add_argument("--top-k", type=int, default=0)
    parser.add_argument("--top-p", type=float, default=0.9)
    parser.add_argument("--sample-count", type=int, default=1)
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    if args.database and (not args.user_id or not args.snapshot_id):
        parser.error("--database requires --user-id and --snapshot-id")
    if not args.fixture_paths and not all((args.kronos_root, args.model, args.tokenizer)):
        parser.error("real inference requires --kronos-root, --model, and --tokenizer")
    if args.lookback <= 0 or args.lookback > 512:
        parser.error("--lookback must be between 1 and 512")
    if args.seed < 0 or args.seed > 2**63 - 1:
        parser.error("--seed must be between 0 and 2^63 - 1")
    if args.temperature <= 0 or args.top_k < 0 or not 0 < args.top_p <= 1 or args.sample_count <= 0:
        parser.error("sampling requires temperature > 0, top-k >= 0, 0 < top-p <= 1, and sample-count > 0")
    return args


def main() -> None:
    args = parse_args()
    snapshot = json.loads(args.snapshot_json.read_text()) if args.snapshot_json else load_snapshot(args.database, args.user_id, args.snapshot_id)
    paths = json.loads(args.fixture_paths.read_text()) if args.fixture_paths else generate_paths(snapshot, args)
    rows = transform_paths(snapshot, paths, args.horizons, args.lookback)
    write_archive(args.output, snapshot, rows, args)
    print(args.output)


if __name__ == "__main__":
    main()
