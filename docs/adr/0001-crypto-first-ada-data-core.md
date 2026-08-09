# Build V1 around an internal ada-data-core

_Superseded by ADR 0030. The internal Tauri-independent data-core boundary remains, while its crypto-only V1 scope is replaced._

The previous V1 PRD is superseded and is not a source of truth for this design. V1 starts with public crypto market data in the internal Rust crate `src-tauri/crates/ada-data-core`; account data, credentials, and trading are out of scope. The crate remains independent of Tauri, retrieves and normalizes data without owning persistence, exposes only normalized domain types while keeping provider payloads private, and is reached by the GUI through a thin Tauri adapter exposing `market_list_spot_instruments` and `market_get_bar_series` without any raw provider passthrough. Keeping the crate inside this repository minimizes delivery overhead while preserving extraction when a second real consumer appears.
