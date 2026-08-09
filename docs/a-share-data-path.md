# China A-share data path

[简体中文](a-share-data-path.zh-CN.md)

Issue #72 delivers a Tauri-independent A-share connector in
`adaq-data-core::a_share` and a durable Source → Canonical path in
`adaq-data-pipeline::a_share`.

## Contract

- `akshare-rs` is pinned to `=0.1.14`; the GUI boundary receives only
  asset-neutral DTOs, never provider payloads or local evidence paths.
- Daily/minute/corporate-action wire bytes are retained at the connector
  boundary before normalization so decimal strings and malformed rows are not
  reconstructed from floating-point DTOs.
- The raw-wire adapter is explicitly versioned as
  `adaq-data-core-raw-wire-v1` because the pinned akshare-rs DTOs use floating
  point fields for these methods. Current spot price, base volume, and quote
  volume remain exact decimal strings with the retrieval timestamp.
- Each acquisition retains the connector version, actual upstream and method,
  normalized request parameters, retrieval time, response/content hashes,
  retry diagnostics, capability snapshot, and known limitations.
- Successful acquisition requires matching non-empty raw wire evidence for every
  response hash. A backfill may use only the requesting user's recorded calendar
  snapshot, and the durable source publication is idempotent across a crash
  between catalog cutover and checkpoint completion.
- Instrument IDs are exchange identities (`sse`/`szse` plus the six-digit
  code). Conflicting provider prefixes are rejected. Master snapshots are
  immutable and selected point-in-time for backfills.
- Canonical equity Bars are `Unadjusted`. Adjusted values are not silently
  mixed into the canonical series; corporate actions are separate immutable
  evidence revisions.
- UTC instants are derived from Asia/Shanghai. The calendar records 09:30–11:30
  and 13:00–15:00 sessions, the 11:30–13:00 break, weekends, holidays, and
  provider limitations around early/ad-hoc closures.

## Provider methods

| Evidence | akshare-rs method | Upstream |
| --- | --- | --- |
| Instrument master/current values | `stock_zh_a_spot` + raw-wire `Market_Center.getHQNodeData` | Sina Finance |
| Daily Bars | raw-wire `stock_zh_a_daily` with `adjust=""` | Eastmoney kline |
| Intraday Bars | raw-wire `stock_zh_a_minute` | Sina KLineData |
| Corporate actions | raw-wire `stock_fhps_detail_em` | Eastmoney |
| Trading dates | raw-wire `tool_trade_date_hist` | Sina |

The connector uses bounded attempts, request timeouts, retry delay, and
provider-response-window limitations. Out-of-range/open observations are
excluded from the requested closed range; malformed rows are retained as
Source evidence and reach the canonicalizer for quarantine. Corporate-action
rows use the same retained-evidence boundary and are marked `Passed`,
`Degraded`, or `Rejected` without discarding quarantined rows. A publication is
`Passed`, `Degraded`, or `Rejected`; a cancelled or failed backfill leaves its
checkpoint and failure evidence without publishing a partial canonical file,
including cancellation during a retry or timeout. Minute rows without an
upstream exact amount are quarantined rather than having turnover inferred.
Desktop commands expose master acquisition/listing, point-in-time membership,
calendar and corporate-action evidence, bounded backfill/cancellation, and the
asset-neutral workspace DTO. A stale `Running` checkpoint is safe to resume
after restart because acquired bars are checkpointed durably before publication;
no partial Source or Canonical publication is visible.

## Local verification

The committed fixtures under `src-tauri/fixtures/a-share/` are offline-only
and are not trading data. Run the focused checks from `src-tauri`:

```sh
cargo test -p adaq-data-core a_share --lib
cargo test -p adaq-data-pipeline a_share::tests --lib
```

Live provider availability, rate limits, history windows, suspension periods,
and disagreements between upstreams remain evidence limitations rather than
being filled with inferred values.

## Troubleshooting

- An empty daily or minute response is retained with its raw hash and reported
  as unconfirmed availability; it is not converted into synthetic Bars.
- An adjusted payload or malformed row is rejected or quarantined with the raw
  response retained. Inspect the quality report and limitation list before
  retrying.
- A cancelled or failed backfill keeps its checkpoint and acquisition file. A
  later run with the same task request resumes that evidence; a changed request
  requires a new task ID.
- A missing raw file, calendar file, or hash mismatch is a storage-integrity
  failure. Restore the local evidence store or reacquire the snapshot instead
  of bypassing validation.
