# Keep Strategy Candidates separate from qualified Components and Backtests

Status: accepted

Gate 10 creates a User-owned, immutable Strategy Candidate Revision from a canonical Declarative Strategy Definition and exact accepted Factor and Model bindings. A Candidate is a reusable research input with a stable identity, append-only revisions, immutable Scope, and finite parameters; it is not a Strategy Component, Component Package, or Backtest Run. Gate 11 must explicitly consume one exact Revision and produce any qualified deployable Package separately, reusing existing StrategyProject persistence patterns where their semantics match.
