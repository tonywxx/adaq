# Factor EMA 5/10 Crossover

This time-series Factor consumes the frozen `ema-5` and `ema-10` Feature Slots.
On the first bullish crossover it records the EMA5 value at the crossover. On
the next bullish crossover it emits `buy-signal = 1` only when that EMA5 value
is higher; all other delivered rows emit `0`.

The current V1 Feature Dataset does not expose a separate market `high` slot,
so the recorded high is deliberately the EMA5 crossover value. The Factor is
stateful across process batches and returns no output for its first delivered
row.
