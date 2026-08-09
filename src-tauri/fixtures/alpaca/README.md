# Alpaca Basic fixtures

These redacted JSON fixtures are for the Tauri-independent Alpaca connector
tests and local mock-server review. They model the authenticated Market Data
API shape without containing a key pair or provider credential.

The fixtures intentionally identify the feed as `iex`, preserve decimal fields
as strings, include a pagination token, and include a stream update (`u`) so
corrections remain observable rather than silently overwriting Source evidence.
