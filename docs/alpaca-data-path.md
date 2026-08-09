# Alpaca Basic U.S. equity data path

ADAQ's U.S. equity path uses an authenticated Alpaca Market Data Basic
connection. The fixed market-data endpoint is `https://data.alpaca.markets`;
the supported stream is `wss://stream.data.alpaca.markets/v2/iex`.

## Setup

1. Open **Settings > Connections** in the desktop app.
2. Enter the Alpaca **Paper API Key ID** and **Paper Secret Key** there.
3. Save and test the connection.

Never paste either value into chat, source files, logs, diagnostics, pipeline
provenance, or a `.env` file. The Host resolves the key pair from the operating
system secret store; GUI and pipeline DTOs contain only the Profile identity
and masked public-key suffix.

## Basic-plan contract

- Feed: **IEX-only**. ADAQ does not describe it as consolidated or full-market
  realtime data.
- Historical bars: available from the connector's declared 2016 start, with a
  runtime capability cutoff for the latest 15 minutes.
- Historical request control: at most 200 requests per minute, with bounded
  retries and pagination.
- Streaming: one connection and at most 30 symbols for the Basic path;
  reconnects and provider errors are surfaced as stream events.
- Unavailable capabilities are retained in the Provider Capability Snapshot,
  including consolidated realtime, full-market volume, newer-than-15-minute
  historical bars, and provider corporate-action data.

## Evidence boundary

The pipeline retains provider response hashes/raw responses, Instrument Master
revisions, `America/New_York` Trading Calendar Snapshots, UTC session bounds,
Source revisions, Canonical revisions, quality reports, gaps, provider errors,
and coverage limitations. Bars are always **Unadjusted**. Corporate actions
are not merged, used to overwrite bars, or used as gap repair.

Alpaca late/corrected payloads publish a new immutable Source revision. An
optional auxiliary historical/corporate-action cross-check, if added later,
must remain separately labeled and must not become an automatic fallback or
merge into Alpaca Canonical evidence.
