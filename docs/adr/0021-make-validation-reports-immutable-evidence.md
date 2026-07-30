# Make Validation Reports immutable evidence over exact Runs

ADAQ generates each Validation Report from one immutable Validation Protocol and an exact set of immutable Backtest Runs, and the Report references their identities rather than copying mutable summaries. Changing any Package, Snapshot, configuration, time split, validation method, Run, or aggregation rule creates a new Report; this preserves auditability and reproducibility at the cost of retaining superseded Reports, while keeping performance evidence outside Component Meta and `.adaq` Packages.
