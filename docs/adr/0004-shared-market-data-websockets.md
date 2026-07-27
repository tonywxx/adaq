# Share market-data WebSockets per Venue

ADAQ maintains one shared Ticker WebSocket and one shared OHLCV Bar WebSocket per Venue, multiplexes active subscriptions over each connection, and fans updates out to all interested views. Watchlist and Active Instrument subscriptions persist for the signed-in application session rather than following individual view mounts. Historical Bar bootstrap and backfill remain REST-based; each Venue owns separate connections because provider endpoints, protocols, and reconnect behavior differ.
