# adaq-trading-crypto

[![Crates.io](https://img.shields.io/crates/v/adaq-trading-crypto.svg)](https://crates.io/crates/adaq-trading-crypto)
[![docs.rs](https://docs.rs/adaq-trading-crypto/badge.svg)](https://docs.rs/adaq-trading-crypto)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue.svg)](https://blog.rust-lang.org)

A unified trading interface for **crypto and prediction markets**, written in Rust for the
[AdaQ](https://github.com/tonywxx/adaq) quantitative-trading platform.

The API surface and unified data structures are **ccxt-compatible**: the same `snake_case`
method names (`fetch_ohlcv`, `create_order`, …), the same field names, and typed structs that keep
the raw exchange response under an `info` field as an escape hatch.

> 📘 简体中文文档见 [README.zh-CN.md](./README.zh-CN.md).

## Features

- **Unified REST API via the `Exchange` trait** — `snake_case` method and field names, so code
  ports almost line-for-line from other ccxt-compatible clients.
- **109 exchange adapters** — 22 curated (hand-authored) covering the full surface, plus 87
  generated exchanges on the shared unified REST surface.
- **Typed, serde-tolerant data structures** — `Market`, `Ticker`, `OrderBook`, `OHLCV`, `Order`,
  `Trade`, `Position`, `Balance`, … — with `info` carrying the raw response for full fidelity.
- **Exact decimal arithmetic** — `rust_decimal` by default, plus a `Precise` module and a
  `decimal_to_precision` helper matching ccxt's `Precise` / `decimalToPrecision` semantics.
- **Optional synchronous wrapper** (`sync` feature) — use the API without managing an async
  runtime.
- **Optional real-time WebSocket layer** (`realtime` feature) — 8 `watch_*` channels.
- **Prediction-market support** — Kalshi, Polymarket (EIP-712 order signing), Manifold (native),
  plus Limitless / Myriad / Opinion.
- **Apache-2.0** licensed; upstream-derived parsing logic retains its MIT notice (see `NOTICE`).

## Installation

```toml
[dependencies]
adaq-trading-crypto = { version = "1", features = ["binance", "okx"] }
```

Enable only the exchanges you need (each is a Cargo feature), plus optional `realtime` /
`sync`. The default features are `binance`, `okx`, `kalshi`, `polymarket`. Use the `full`
feature to compile everything.

## Quick Start

### Async

```rust
use adaq_trading_crypto::{Config, Exchange};
use adaq_trading_crypto::adapters::Binance;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut cfg = Config::new();
    cfg.api_key = Some("YOUR_KEY".into());
    cfg.secret  = Some("YOUR_SECRET".into());

    let ex = Binance::new(cfg)?;
    let ticker = ex.fetch_ticker("BTC/USDT", Default::default()).await?;
    println!("last = {}", ticker.last.unwrap());
    Ok(())
}
```

### Sync (no runtime to manage)

```rust
use adaq_trading_crypto::sync::BlockingExchange;
use adaq_trading_crypto::{Config, Exchange};
use adaq_trading_crypto::adapters::Binance;

let ex = BlockingExchange::new(Binance::new(Config::new())?)?;
let ticker = ex.fetch_ticker("BTC/USDT", Default::default())?;
println!("last = {}", ticker.last.unwrap());
```

### Prediction market (Polymarket)

```rust
use adaq_trading_crypto::adapters::Polymarket;
use adaq_trading_crypto::{Config, Exchange};

let ex = Polymarket::new(Config::new())?;
let markets = ex.fetch_markets(Default::default()).await?;
```

## Supported Exchanges

The crate ships **109 exchanges** — 22 curated adapters with full coverage, and 87 generated
exchanges on the shared unified REST surface. Each exchange is enabled by its own Cargo feature
(the feature name matches the exchange id shown below).

### Curated adapters (22)

These are hand-authored and cover the complete method surface, including signing and
exchange-specific parsing.

| Feature       | Exchange        | Scope                                                                |
| ------------- | --------------- | -------------------------------------------------------------------- |
| `binance`     | Binance         | REST public + private, WS 8 channels                                 |
| `okx`         | OKX             | REST public + private, WS 8 channels                                 |
| `bybit`       | Bybit           | REST public + private, WS 8 channels                                 |
| `kraken`      | Kraken          | REST public + private, WS 8 channels (no `positions` channel)        |
| `coinbase`    | Coinbase        | REST public + private (v3 Advanced Trade)                            |
| `bitget`      | Bitget          | REST public + private                                                |
| `gate`        | Gate.io         | REST public + private                                                |
| `mexc`        | MEXC            | REST public + private                                                |
| `htx`         | HTX (Huobi)     | REST public + private                                                |
| `kucoin`      | KuCoin          | REST public + private                                                |
| `hyperliquid` | Hyperliquid     | DEX: public market data (markets / tickers / ohlcv / order_book)     |
| `kalshi`      | Kalshi          | REST full (incl. order placement), WS private channels               |
| `polymarket`  | Polymarket      | REST + EIP-712 order signing, WS 5 channels                          |
| `manifold`    | Manifold        | Native adapter: markets / ticker / trades                          |

> **New in v1.0.3 ([ADR-0017](docs/adr/0017-hybrid-evolution-curated-generated-boundary.md)):**
> 8 adapters — `alpaca`, `aster`, `binanceus`, `gemini`, `hashkey`, `lighter`, `myokx`, `okxus` —
> were promoted from generated to curated, giving them full REST coverage (public + private/trading).
> The curated set is now **22**.

| `alpaca`      | Alpaca          | REST public + private (crypto markets)                             |
| `aster`       | Aster           | REST public + private (DEX)                                        |
| `binanceus`   | Binance US      | REST public + private                                              |
| `gemini`      | Gemini          | REST public + private                                              |
| `hashkey`     | HashKey         | REST public + private                                              |
| `lighter`     | Lighter         | DEX: REST public + private (EIP-712 on-chain orders)               |
| `myokx`       | OKX             | REST public + private                                              |
| `okxus`       | OKX US          | REST public + private                                              |

### Generated exchange coverage (87)

The remaining exchanges share the same unified REST surface and are enabled by their individual
Cargo feature. They cover the common REST methods — `fetch_markets`, `fetch_ticker`,
`fetch_ohlcv`, `fetch_order_book`, `fetch_trades`, `fetch_balance`, `fetch_orders`,
`create_order`, `cancel_order`, … — through the shared engine. Methods that need
exchange-specific signing stay `NotSupported`.

| Feature            | Feature               | Feature             |
| ------------------ | --------------------- | ------------------- |
| `apex`             | `backpack`            | `bequant`           |
| `bigone`           | `binancecoinm`        | `binanceusdm`       |
| `bingx`            | `bit2c`               | `bitbank`           |
| `bitbns`           | `bitfinex`            | `bitflyer`          |
| `bithumb`          | `bitmex`              | `bitopro`           |
| `bitrue`           | `bitso`               | `bitstamp`          |
| `bitteam`          | `bittrade`            | `bitvavo`           |
| `blockchaincom`    | `blofin`              | `btcbox`            |
| `btcmarkets`       | `btcturk`             | `bullish`           |
| `bybiteu`          | `bydfi`               | `cex`               |
| `coinbaseexchange` | `coinbaseinternational` | `coincheck`         |
| `coinex`           | `coinmate`            | `coinone`           |
| `coinsph`          | `coinspot`            | `cryptocom`         |
| `cryptomus`        | `deepcoin`            | `delta`             |
| `deribit`          | `derive`              | `digifinex`         |
| `dydx`             | `exmo`                | `extended`          |
| `fmfwio`           | `foxbit`              | `gateeu`            |
| `grvt`             | `hibachi`             | `hitbtc`            |
| `hollaex`          | `independentreserve`  | `indodax`           |
| `krakenfutures`    | `kucoinfutures`       | `latoken`           |
| `lbank`            | `luno`                | `mercado`           |
| `modetrade`        | `mudrex`              | `nado`              |
| `ndax`             | `onetrading`          | `p2b`               |
| `pacifica`         | `paradex`             | `paymium`           |
| `phemex`           | `poloniex`            | `tokocrypto`        |
| `toobit`           | `upbit`               | `weex`              |
| `whitebit`         | `woo`                 | `woofipro`          |
| `xt`               | `zaif`                | `zebpay`            |
| `limitless`        | `myriad`              | `opinion`           |

The three long-tail prediction markets — **Limitless / Myriad / Opinion** — are part of this
generated set, while Kalshi / Polymarket / Manifold remain among the curated adapters above.

## Feature Flags

| Feature           | Effect                                                                 |
| ----------------- | ---------------------------------------------------------------------- |
| per-exchange      | Compile in that adapter (e.g. `binance`, `kraken`, `polymarket`, …)    |
| `realtime`        | WebSocket `watch_*` methods (the 8 channels below)                     |
| `sync`            | Blocking wrapper over the async API (`sync::BlockingExchange`)         |
| `prediction`      | `kalshi` + `polymarket` + `manifold` (curated prediction markets)      |
| `full`            | All exchanges (curated + generated) + `realtime`                        |
| _default_         | `binance`, `okx`, `kalshi`, `polymarket`                               |

## Architecture

Adapters follow the **HttpCore + four-seam** model (ADR-0013):

- **`HttpCore`** — an exchange-agnostic deep module that owns the HTTP request skeleton, market
  caching, client-side pagination/filtering, safe field extraction, and `Precise` arithmetic. It
  also hosts normalized helpers — `iso8601_ms`, `parse_ohlcv_standard` — and the shared
  error-code → `ErrorKind` mapping (`ERROR_CODE_MAP`, the ADR-0013 `handle_errors` seam).
- **`signing`** — a shared deep module ([`src/signing.rs`](src/signing.rs), the ADR-0013 `sign`
  seam) that owns every exchange-agnostic signing primitive — HMAC (SHA-256/384/512, hex &
  base64), recoverable ECDSA (Polymarket EIP-712), and RSA-PSS (Kalshi) — credential extraction
  (`require_api_key` / `require_secret` / `require_passphrase`), and panic-free header assembly
  (`set_header`). Each adapter's `private_request` keeps only the exchange-specific auth-string
  concatenation and header-name set.
- **Four seams** — each adapter only fills: `describe` (endpoint paths/parameters), `sign`
  (signing algorithm), `handle_errors` (error-code mapping), and field mapping (`parse`
  overrides). The generated exchange adapters fill only the `describe` seam and reuse `HttpCore`
  unchanged.

> **New in v1.0.4:** a consolidation release with no API-surface or exchange changes. The `sign`
> seam moved into a shared `src/signing.rs` deep module, `HttpCore` absorbed normalized time/OHLCV
> helpers and the shared error-code map, and `generic.rs`'s generated-path `parse_*` consumers were
> split into `src/generic_parse.rs` to isolate the ADR-0016 contract anchors. ~330 lines were
> removed across the curated adapters; behavior and fixtures are unchanged.

> **New in v1.0.5 — unified signing consolidation (no API-surface or exchange changes):** the
> ADR-0013 `sign` seam is now a single shared deep module. `src/signing.rs` consolidates every
> exchange-agnostic signing primitive — HMAC (SHA-256/384/512), recoverable ECDSA (Polymarket
> EIP-712), and RSA-PSS (Kalshi) — under one testable surface. ECDSA signing moved out of
> `eip712.rs` (now EIP-712 digest/encoding only), RSA-PSS moved out of `kalshi.rs`, and Kraken's
> inline HMAC-SHA512 now reuses the shared `hmac_sha512_b64_bytes` byte-key variant. The release
> workflow now fails loudly if every publish retry fails. Behavior and fixtures are unchanged.

The 109 exchanges follow a **hybrid evolution model** ([ADR-0017](docs/adr/0017-hybrid-evolution-curated-generated-boundary.md)):
the **22 curated** adapters are hand-authored with the full trading surface (team-maintained,
optimizable), while the **87 generated** adapters are transpiled from ccxt `describe()` and cover
the public surface only (maintained upstream by ccxt). `Promote` is a repeatable operation to move
any long-tail exchange from generated to curated when full trading is needed.

Prediction-market specifics (outcome context, synthetic tickers, RSA-PSS / EIP-712 / ECDSA
signing) live in the adapters, never in the core.

The unified data structures in `types` use `snake_case` field names compatible with ccxt and keep
the raw response under `info` for full fidelity.

## Real-time (WebSocket)

Enable `realtime` to use the 8 `watch_*` channels on the `Realtime` trait:
`watch_ticker`, `watch_order_book`, `watch_trades`, `watch_ohlcv`, `watch_balance`,
`watch_orders`, `watch_my_trades`, `watch_positions` (per ADR-0009). The `watch_*` layer reuses
the REST `parse_*` parsers (ADR-0015) and shares an incremental order-book engine. The `sync`
feature covers REST only; the WebSocket layer requires async.

## Numerical Accuracy

Prices and quantities use exact decimal arithmetic (`rust_decimal` by default). A public `precise`
module mirrors ccxt's `Precise` semantics, and `decimal_to_precision(value, mode, precision,
rounding, padding)` replicates ccxt's `decimalToPrecision` (decimal places / significant digits /
tick size; round / truncate / up / down).

## Error Handling

A unified `Error` wraps an `ErrorKind` that mirrors ccxt's exception tree, carrying context,
retryability, and `From` conversions for ergonomic `?` propagation. Exchange error codes are now
classified into fine-grained `ErrorKind` variants via a shared `ERROR_CODE_MAP` (the ADR-0013
`handle_errors` seam): for example Binance `-1003` → `RateLimitExceeded` (retryable), `-1121` →
`BadSymbol`, `-2015` → `Authentication`; Bybit `10001` → `InvalidOrder`, `10003` →
`Authentication`; HTX `invalid-symbol` → `BadSymbol`. Unmapped codes fall back to
`ErrorKind::Exchange`, preserving prior behavior.

## Testing & Quality

- **Contract tests** — the unified method surface is extracted from the `Exchange` / `Realtime`
  traits at build time (see `build.rs`) and asserted for completeness; this is a release gate.
- **Differential tests** — parsers are compared field-by-field against real ccxt output recorded
  from identical raw responses (offline, deterministic fixtures in CI).
- `clippy -D warnings`, `rustfmt`, MSRV **1.85** (edition **2024**), `cargo nextest`.

## Development

Regenerate the additional exchange adapters. The generator reads each exchange's `describe()` from
a ccxt Python package and emits per-exchange modules under `src/adapters/generated/`; the shared
runtime lives in `src/generic.rs`.

`gen_adapters.py` takes **no `--ccxt` flag**. It prefers a pip-installed ccxt (pin
`ccxt==4.5.73`) and only falls back to a repo-local `ccxt/python` checkout as an offline option:

```bash
pip install "ccxt==4.5.73"

# Full regen (all ~87 transpiled exchanges):
python3 scripts/gen_adapters.py

# Regenerate a single exchange:
python3 scripts/gen_adapters.py --only=<id>     # e.g. --only=binance
```

After regenerating, run `cargo fmt --all` and commit the generated layer
(`src/adapters/generated/`, `src/adapters/generated.rs`, `Cargo.toml`) — the CI
`transpiler-fresh` job fails if it is stale. The transpiler's contract with the Rust engine
(`src/generic.rs`) is guarded by `python3 scripts/test_sync.py`.

## Changelog

See [CHANGELOG.md](./CHANGELOG.md) for the full release history (bilingual: English + 中文).

## License

Apache-2.0. Parsing logic derived from the upstream library retains its MIT notice — see `NOTICE`.

---

📘 简体中文文档见 [README.zh-CN.md](./README.zh-CN.md).
