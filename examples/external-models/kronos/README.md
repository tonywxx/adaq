# External Kronos Adapter

[English](README.md) | [简体中文](README.zh-CN.md)

This reference path runs `Kronos-small` with `Kronos-Tokenizer-base` in a researcher-managed Python environment and converts generated OHLCV paths into canonical AdaQ Forecast Signals. The Tokenizer only encodes and decodes K-lines; it is not an inference model. `Kronos-small` performs autoregressive inference.

AdaQ never loads Python, PyTorch, Hugging Face code, downloaded weights, or arbitrary external code in the desktop process. The Adapter writes a bounded `.adaq-signals` archive; AdaQ validates and imports that evidence as **Externally Generated**. It is not Verified inference and is not Marketplace-ready.

## Supported environment and exact upstream inputs

The documented environment is Python 3.10–3.12 on 64-bit macOS, Linux, or Windows, with CPU, CUDA, or MPS selected explicitly. A GPU is optional. Create an isolated environment and install the pinned Adapter dependencies:

```sh
cd examples/external-models/kronos
python3.12 -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install -r requirements.txt
```

Acquire these exact upstream revisions:

| Artifact | Official source | Revision |
| --- | --- | --- |
| Kronos source and preprocessing | `https://github.com/shiyu-coder/Kronos` | `67b630e67f6a18c9e9be918d9b4337c960db1e9a` |
| `Kronos-small` model and weights | `NeoQuasar/Kronos-small` | `901c26c1332695a2a8f243eb2f37243a37bea320` |
| `Kronos-Tokenizer-base` and weights | `NeoQuasar/Kronos-Tokenizer-base` | `0e0117387f39004a9016484a186a908917e22426` |

```sh
git clone https://github.com/shiyu-coder/Kronos.git upstream/Kronos
git -C upstream/Kronos checkout 67b630e67f6a18c9e9be918d9b4337c960db1e9a
hf download NeoQuasar/Kronos-small --revision 901c26c1332695a2a8f243eb2f37243a37bea320 --local-dir artifacts/Kronos-small
hf download NeoQuasar/Kronos-Tokenizer-base --revision 0e0117387f39004a9016484a186a908917e22426 --local-dir artifacts/Kronos-Tokenizer-base
shasum -a 256 artifacts/Kronos-small/model.safetensors artifacts/Kronos-Tokenizer-base/model.safetensors upstream/Kronos/model/kronos.py
```

Before use, inspect `upstream/Kronos/LICENSE` and the `LICENSE`/model-card metadata at both pinned Hugging Face revisions. They currently declare MIT; the researcher remains responsible for checking the exact downloaded terms and fitness for the intended use. The Adapter hashes both weight and configuration files, upstream preprocessing, itself, and the complete inference/runtime configuration into provenance.

## Snapshot-aligned input

1. In AdaQ, download and freeze the exact Market Data Snapshot, then copy its Snapshot ID from the research UI.
2. In Settings → General, choose **Open Data Folder**, then quit AdaQ so the read-only external process sees a stable database and Parquet file.
3. Identify the owning User ID if needed:

   ```sh
   sqlite3 /path/to/data/adaq.db 'select distinct user_id from market_data_snapshot_access order by user_id;'
   ```

The Adapter opens `adaq.db` read-only, checks the `(User, Snapshot ID)` access row, and reads the exact Snapshot Parquet referenced by its immutable metadata. It never substitutes a latest or approximately matching series.

## Forecast configuration and deterministic Seed

Run from this directory:

```sh
python adapter.py \
  --database /path/to/data/adaq.db \
  --user-id YOUR_USER_ID \
  --snapshot-id YOUR_SNAPSHOT_ID \
  --kronos-root upstream/Kronos \
  --model artifacts/Kronos-small \
  --tokenizer artifacts/Kronos-Tokenizer-base \
  --lookback 400 \
  --horizons 1,6,24 \
  --temperature 1.0 --top-k 0 --top-p 0.9 --sample-count 1 \
  --seed 7 --device cpu \
  --output kronos-small.adaq-signals
```

`lookback` is limited to Kronos-small's 512-Bar context. The Adapter applies the Seed to Python, NumPy, PyTorch, and all CUDA devices before ordered inference and records it with temperature, top-k, top-p, sample count, device, revisions, and hashes. External GPU kernels and upstream libraries may still vary across hardware or versions, which is one reason the result remains Externally Generated. Preserve the environment and output hashes when comparing reruns.

## Canonical transformation

For every Closed Bar in the Snapshot, the Adapter keeps the original row identity and emits one ordered Signal frame:

| Field | Rule |
| --- | --- |
| Prediction Time | The origin Closed Bar's close boundary, in Unix milliseconds |
| Signal name | `expected-close-return-H-bar` for declared horizon `H` |
| Aggregation | `generated_close[H - 1] / origin_close - 1` |
| Units and Value Scale | Decimal return (`0.01` means 1%), native scale |
| Forecast Target | Built-in continuous `future-close-return` at exactly `H` Bars |
| `availableAt` | Prediction Time under `closed-bar@1`; Strategy execution can occur no earlier than the next Bar |
| Warmup | Unavailable until `lookback` contiguous Closed Bars exist |
| Bar Gap | Starts a new context; Warmup restarts and no context crosses the gap |
| MissingInput | Unavailable when the required generated path is absent; never replaced with zero or a shifted horizon |

Generated future OHLCV rows are not imported as realized market data. Only the aggregated Forecast Signals enter `signals.parquet`.

The canonical manifest records the exact Model Artifact identity, model and Tokenizer weight hashes and revisions, Adapter and preprocessing hashes, complete inference configuration, Seed, upstream sources, licence statement, availability policy, Snapshot, and Producer Segment. Upstream training/fitting/validation boundaries are recorded as unknown when the exact evidence is unavailable; unknown facts are never inferred.

## Import, inspect, evaluate, and Backtest

1. Reopen AdaQ. In **Models → Create Dataset**, import `kronos-small.adaq-signals`.
2. In **Signal Datasets**, confirm the exact Snapshot, signal contracts, row statuses, hashes, Producer Segment, provenance, and **Externally Generated** trust state.
3. In **Evaluation Reports**, select a compatible Expected Value signal and an evaluation window, create the immutable Forecast Evaluation Report, inspect coverage/missingness and expected-value metrics, then export JSON or Markdown.
4. `Unknown` Evaluation Evidence State means complete upstream training, fitting, or normalization-window evidence is missing. It is usable local research evidence, but it is not out-of-sample, Verified, or a performance guarantee.
5. Import or select a Strategy whose Forecast Signal Slot exactly matches Prediction Kind, Target, horizon, and native scale. In **Backtest**, select the same Snapshot and bind the imported Dataset signal.
6. Run the Dataset-first Backtest. AdaQ never reruns Kronos implicitly. Inspect the Run's frozen Feature Plan, Dataset Lock, Producer provenance, Unknown evidence state, pauses, decisions, fills, and metrics.

Forecast Evaluation measures predictions. A Backtest and any downstream Validation Report measure Strategy behavior; neither changes the external inference trust state.

## Deterministic automated fixture

The small fixture uses recorded generated paths and never downloads weights or requires a GPU:

```sh
python -m unittest test_adapter.py
python adapter.py --snapshot-json fixtures/snapshot.json --fixture-paths fixtures/generated-paths.json \
  --lookback 2 --horizons 1,2 --seed 7 --output /tmp/kronos-fixture.adaq-signals
```

Rust tests use the same fixture to exercise transformation literals, manifest, Parquet, archive import, Unknown evaluation evidence, compatible Strategy import, Dataset-first Backtest, and Run Dataset Lock.

## Optional real-inference evidence and troubleshooting

Real inference is optional because downloads, memory, runtime, and accelerator support vary. Capture this evidence before retrying or reporting a failure:

- Download: command, URL/repository, exact revision, HTTP/error output, file size, and SHA-256.
- Runtime or memory: OS, CPU/GPU, accelerator/runtime versions, `python --version`, `python -m pip freeze`, command, Seed/configuration, peak memory, elapsed time, and full traceback.
- Schema or transformation: Snapshot ID, User ID (redact externally), instrument/interval, Bar count/gaps, horizons/lookback, offending Prediction Time, and generated-path shape; do not attach private market data unless intended.
- Import: `.adaq-signals` SHA-256/size, manifest (after reviewing sensitive paths), exact AdaQ error, and whether any Dataset appeared.
- Evaluation: Dataset ID, signal/horizon/window, Report ID if created, Evidence State, unavailable rows, and export/error.
- Backtest: Strategy package hash, Snapshot/Dataset/signal binding, preflight error or Run ID, Dataset Lock, pauses, and relevant export.

Common fixes: a Snapshot mismatch requires regenerating from that exact Snapshot; `warmup` requires a longer contiguous segment or smaller documented lookback; `missing-input` requires a complete generated path; hash/schema errors require preserving the failed archive and regenerating rather than editing it; out-of-memory requires a smaller lookback/sample count or CPU, with the changed configuration recorded as new evidence.

## Future Qlib boundary

Microsoft Qlib may later prepare training data, train or fine-tune Artifacts, and export predictions through the same External Model Adapter boundary. M8 does not include Qlib integration, training, a controlled Python Runner, Verified external inference, live execution, or Marketplace publishing. A future controlled Runner or verified AdaQ Model Component must add its own execution evidence without weakening the `.adaq-signals` provenance and Dataset-first boundary.
