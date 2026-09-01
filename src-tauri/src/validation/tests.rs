//! Interface-level Validation Studies tests (real SQLite + temp directories).
//!
//! These tests exercise the module exclusively through its public
//! interface — Protocol creation and listing, Report running, listing, and
//! export, plus the summary, reset, and run-reference hooks — on a real
//! LocalResearchState composition root.

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use adaq_backtest_core::{ExecutionProfile, MarketDataSnapshot};
use adaq_data_core::{BarGap, BarInterval, BarSeries, OhlcvBar};
use rust_decimal::Decimal;

use super::{
    CrossMarketValidationContextRequest, CrossMarketValidationRequest,
    ValidationProtocolCreateRequest, ValidationWindowRequest, WalkForwardValidationRequest,
};
use crate::{
    backtest::{BacktestRunRequest, FactorInstanceRequest},
    local_research::{LocalDataResetKind, LocalResearchState},
};

struct Harness {
    root: PathBuf,
    state: Arc<LocalResearchState>,
    /// Present so the Watchlist schema exists for the Reset flows.
    #[allow(dead_code)]
    watchlist: crate::watchlist::WatchlistDb,
    snapshot: MarketDataSnapshot,
    strategy_hash: String,
    factor_hash: String,
}

fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "adaq-validation-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn public_example_package(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/components")
        .join(name)
        .join("dist")
        .join(format!("{name}-0.1.0.adaq"));
    assert!(
        path.is_file(),
        "build the {name} example with adaq-component build"
    );
    fs::read(path).unwrap()
}

/// Builds the deterministic public-example Backtest environment: one hourly
/// BTC-USDT Snapshot of 50 Closed Bars with a 5-hour gap, plus the imported
/// momentum Factor and trend Strategy Components.
fn harness(name: &str) -> Harness {
    let root = root(name);
    let state = LocalResearchState::open(&root).unwrap();
    let watchlist = crate::watchlist::WatchlistDb::open(&root.join("adaq.db")).unwrap();
    let factor_bytes = public_example_package("factor-close-momentum-5");
    let factor_hash = adaq_component_tooling::ComponentPackage::read(&factor_bytes)
        .unwrap()
        .archive_sha256;
    let strategy_bytes = public_example_package("strategy-momentum-trend");
    let strategy_hash = adaq_component_tooling::ComponentPackage::read(&strategy_bytes)
        .unwrap()
        .archive_sha256;
    state.components.import("alice", &factor_bytes).unwrap();
    state.components.import("alice", &strategy_bytes).unwrap();
    let bars = (0..50)
        .map(|index| {
            let close = Decimal::from(100 + index);
            let time_index = if index < 25 { index } else { index + 5 };
            OhlcvBar {
                open_time_ms: time_index * 3_600_000,
                open: close,
                high: close,
                low: close,
                close,
                base_volume: Decimal::ONE,
                quote_volume: close,
            }
        })
        .collect();
    let snapshot = state
        .persist_snapshot_for_user(
            "alice",
            &BarSeries {
                src: "okx".into(),
                code: "BTC-USDT".into(),
                interval: BarInterval::OneHour,
                bars,
                gaps: vec![BarGap {
                    start_time_ms: 25 * 3_600_000,
                    end_time_ms: 30 * 3_600_000,
                }],
            },
        )
        .unwrap();
    Harness {
        root,
        state,
        watchlist,
        snapshot,
        strategy_hash,
        factor_hash,
    }
}

fn finish(harness: Harness) {
    drop(harness.watchlist);
    drop(harness.state);
    fs::remove_dir_all(harness.root).unwrap();
}

fn run_request(harness: &Harness) -> BacktestRunRequest {
    BacktestRunRequest {
        user_id: "alice".into(),
        snapshot_id: harness.snapshot.snapshot_id.clone(),
        portfolio_universe_snapshot_id: None,
        run_start_time_ms: None,
        run_end_time_ms: None,
        factor_instances: vec![FactorInstanceRequest {
            alias: "momentum".into(),
            archive_sha256: harness.factor_hash.clone(),
            parameters: HashMap::new(),
        }],
        signal_instances: vec![],
        strategy_archive_sha256: harness.strategy_hash.clone(),
        strategy_parameters: HashMap::new(),
        initial_quote_allocation: 10_000.into(),
        execution_profile: ExecutionProfile {
            maker_fee_rate: Decimal::new(8, 4),
            taker_fee_rate: Decimal::new(1, 3),
            adverse_slippage_rate: Decimal::ZERO,
            rebalance_threshold: Decimal::ZERO,
            price_increment: Decimal::ONE,
            quantity_increment: Decimal::new(1, 4),
            minimum_quantity: Decimal::new(1, 4),
            risk_free_rate: Decimal::ZERO,
            fill_policy: adaq_backtest_core::FillPolicy::Taker,
        },
        strategy_binding: None,
        risk_policy: None,
        seed: 0,
    }
}

fn holdout_request(harness: &Harness) -> ValidationProtocolCreateRequest {
    ValidationProtocolCreateRequest {
        user_id: "alice".into(),
        run: run_request(harness),
        windows: vec![ValidationWindowRequest {
            snapshot_id: harness.snapshot.snapshot_id.clone(),
            sample_out_start_time_ms: 25 * 3_600_000,
            sample_out_end_time_ms: None,
            sample_in_start_time_ms: None,
            sample_in_end_time_ms: None,
        }],
        walk_forward: None,
        cross_market: None,
        method_version: "chronological-holdout@1".into(),
        aggregation_rule_version: "equal-window@1".into(),
        strategy_binding: None,
        final_evidence_sealed: false,
    }
}

#[test]
fn portfolio_holdout_split_keeps_inclusive_end_bar() {
    let harness = harness("portfolio-holdout-split");
    let window = ValidationWindowRequest {
        snapshot_id: harness.snapshot.snapshot_id.clone(),
        sample_out_start_time_ms: 25 * 3_600_000,
        sample_out_end_time_ms: Some(harness.snapshot.end_time_ms),
        sample_in_start_time_ms: None,
        sample_in_end_time_ms: Some(24 * 3_600_000),
    };

    let (sample_in, sample_out) = harness
        .state
        .validation
        .split_snapshot("alice", &window, true)
        .unwrap();

    assert_eq!(sample_in.end_time_ms, 24 * 3_600_000);
    assert_eq!(sample_out.end_time_ms, harness.snapshot.end_time_ms);
    finish(harness);
}

fn backtest_run_count(state: &LocalResearchState, user_id: &str) -> i64 {
    state.backtests.summary_for_user(user_id).unwrap().run_count as i64
}

#[test]
fn protocol_creation_is_validated_frozen_and_user_scoped() {
    let harness = harness("protocol-create");

    // An empty or non-chronological sample-out window is rejected.
    assert!(
        harness
            .state
            .validation
            .create_protocol(ValidationProtocolCreateRequest {
                windows: vec![ValidationWindowRequest {
                    snapshot_id: harness.snapshot.snapshot_id.clone(),
                    sample_out_start_time_ms: 0,
                    sample_out_end_time_ms: None,
                    sample_in_start_time_ms: None,
                    sample_in_end_time_ms: None,
                }],
                ..holdout_request(&harness)
            })
            .is_err()
    );
    // A Run configuration belonging to another User is rejected.
    assert!(
        harness
            .state
            .validation
            .create_protocol(ValidationProtocolCreateRequest {
                run: BacktestRunRequest {
                    user_id: "bob".into(),
                    ..run_request(&harness)
                },
                ..holdout_request(&harness)
            })
            .is_err()
    );
    // An unknown aggregation rule family is rejected.
    assert!(
        harness
            .state
            .validation
            .create_protocol(ValidationProtocolCreateRequest {
                aggregation_rule_version: "weighted-window@1".into(),
                ..holdout_request(&harness)
            })
            .is_err()
    );
    // A Run referencing an unavailable Component is rejected.
    assert!(
        harness
            .state
            .validation
            .create_protocol(ValidationProtocolCreateRequest {
                run: BacktestRunRequest {
                    strategy_archive_sha256: "0".repeat(64),
                    ..run_request(&harness)
                },
                ..holdout_request(&harness)
            })
            .is_err()
    );
    // Cross-market studies require at least two markets.
    assert!(
        harness
            .state
            .validation
            .create_protocol(ValidationProtocolCreateRequest {
                windows: vec![],
                cross_market: Some(CrossMarketValidationRequest {
                    contexts: vec![CrossMarketValidationContextRequest {
                        snapshot_id: harness.snapshot.snapshot_id.clone(),
                        run_override: None,
                    }],
                }),
                method_version: "cross-market@1".into(),
                ..holdout_request(&harness)
            })
            .is_err()
    );

    let protocol = harness
        .state
        .validation
        .create_protocol(holdout_request(&harness))
        .unwrap();
    assert!(!protocol.protocol_id.is_empty());
    assert_eq!(protocol.method_version, "chronological-holdout@1");
    // Creating the identical request again freezes to the same identity.
    let repeated = harness
        .state
        .validation
        .create_protocol(holdout_request(&harness))
        .unwrap();
    assert_eq!(repeated.protocol_id, protocol.protocol_id);
    assert_eq!(
        harness
            .state
            .validation
            .list_protocols("alice")
            .unwrap()
            .len(),
        1
    );
    assert!(
        harness
            .state
            .validation
            .list_protocols("bob")
            .unwrap()
            .is_empty()
    );
    assert!(
        harness
            .state
            .validation
            .run_report("bob", &protocol.protocol_id)
            .is_err()
    );

    finish(harness);
}

#[test]
fn walk_forward_reports_derive_windows_and_reuse_runs() {
    let harness = harness("walk-forward");
    let walk_forward = WalkForwardValidationRequest {
        snapshot_id: harness.snapshot.snapshot_id.clone(),
        window_size_bars: 5,
        step_size_bars: 5,
        minimum_history_bars: 10,
    };
    let create = |walk_forward: WalkForwardValidationRequest| {
        harness
            .state
            .validation
            .create_protocol(ValidationProtocolCreateRequest {
                windows: vec![],
                walk_forward: Some(walk_forward),
                method_version: "walk-forward@1".into(),
                ..holdout_request(&harness)
            })
    };

    // Walk-forward must split the exact frozen Snapshot.
    assert!(
        create(WalkForwardValidationRequest {
            snapshot_id: "other".into(),
            ..walk_forward.clone()
        })
        .is_err()
    );
    // Steps must not overlap sample-out windows.
    assert!(
        create(WalkForwardValidationRequest {
            step_size_bars: 4,
            ..walk_forward.clone()
        })
        .is_err()
    );
    // History must exceed the minimum.
    assert!(
        create(WalkForwardValidationRequest {
            minimum_history_bars: 50,
            ..walk_forward.clone()
        })
        .is_err()
    );

    let protocol = create(walk_forward.clone()).unwrap();
    // The 5-hour Snapshot gap shifts Bar-open boundaries after index 25.
    assert_eq!(
        protocol
            .windows
            .iter()
            .map(|window| window.sample_out_start_time_ms)
            .collect::<Vec<_>>(),
        vec![
            10 * 3_600_000,
            15 * 3_600_000,
            20 * 3_600_000,
            30 * 3_600_000,
            35 * 3_600_000,
            40 * 3_600_000,
            45 * 3_600_000,
            50 * 3_600_000
        ]
    );
    assert_eq!(
        protocol.windows[0].sample_out_end_time_ms,
        Some(15 * 3_600_000)
    );

    // A single-window tail protocol runs deterministically and reuses Runs.
    let resumable = create(WalkForwardValidationRequest {
        minimum_history_bars: 45,
        ..walk_forward.clone()
    })
    .unwrap();
    let first = harness
        .state
        .validation
        .run_report("alice", &resumable.protocol_id)
        .unwrap();
    let run_count = backtest_run_count(&harness.state, "alice");
    let resumed = harness
        .state
        .validation
        .run_report("alice", &resumable.protocol_id)
        .unwrap();
    assert_eq!(first.report_id, resumed.report_id);
    assert_eq!(backtest_run_count(&harness.state, "alice"), run_count);
    assert_eq!(first.windows.len(), 1);
    assert_eq!(first.windows[0].sample_out_start_time_ms, 50 * 3_600_000);
    assert!(first.windows[0].sample_out_end_time_ms.is_none());
    assert!(first.windows[0].sample_in_run_id.is_some());
    assert!(first.windows[0].sample_out_run_id.is_some());
    assert_eq!(first.aggregate.completed_windows, 1);
    assert_eq!(first.aggregate.failed_windows, 0);
    assert!(
        harness
            .state
            .validation
            .export_report("alice", &first.report_id, "markdown")
            .unwrap()
            .contains("walk-forward@1")
    );

    // The full eight-window study completes every window.
    let full = harness
        .state
        .validation
        .run_report("alice", &protocol.protocol_id)
        .unwrap();
    assert_eq!(full.windows.len(), 8);
    assert_eq!(full.aggregate.completed_windows, 8);
    assert_eq!(full.aggregate.failed_windows, 0);

    // A Run bound to a missing Signal Dataset records failed windows instead
    // of aborting the study.
    let unavailable_run = BacktestRunRequest {
        signal_instances: vec![crate::backtest::SignalInstanceRequest {
            slot: "forecast".into(),
            dataset_id: "missing-dataset".into(),
            signal_name: "up".into(),
        }],
        ..run_request(&harness)
    };
    let failing_protocol = harness
        .state
        .validation
        .create_protocol(ValidationProtocolCreateRequest {
            run: unavailable_run,
            windows: vec![],
            walk_forward: Some(WalkForwardValidationRequest {
                minimum_history_bars: 45,
                ..walk_forward
            }),
            method_version: "walk-forward@1".into(),
            ..holdout_request(&harness)
        })
        .unwrap();
    let unavailable_report = harness
        .state
        .validation
        .run_report("alice", &failing_protocol.protocol_id)
        .unwrap();
    assert_eq!(unavailable_report.aggregate.failed_windows, 1);
    assert!(
        unavailable_report
            .windows
            .iter()
            .all(|window| window.failure.is_some())
    );

    finish(harness);
}

#[test]
fn cross_market_reports_preserve_order_failures_and_identity() {
    let harness = harness("cross-market");
    let (_, bars) = harness
        .state
        .snapshot_for_user("alice", &harness.snapshot.snapshot_id)
        .unwrap();
    let eth_snapshot = harness
        .state
        .persist_snapshot_for_user(
            "alice",
            &BarSeries {
                src: "okx".into(),
                code: "ETH-USDT".into(),
                interval: BarInterval::OneHour,
                bars: bars.clone(),
                gaps: vec![],
            },
        )
        .unwrap();
    let contexts = vec![
        CrossMarketValidationContextRequest {
            snapshot_id: harness.snapshot.snapshot_id.clone(),
            run_override: None,
        },
        CrossMarketValidationContextRequest {
            snapshot_id: eth_snapshot.snapshot_id.clone(),
            run_override: None,
        },
    ];
    let create = |contexts: Vec<CrossMarketValidationContextRequest>| {
        harness
            .state
            .validation
            .create_protocol(ValidationProtocolCreateRequest {
                windows: vec![],
                cross_market: Some(CrossMarketValidationRequest { contexts }),
                method_version: "cross-market@1".into(),
                ..holdout_request(&harness)
            })
    };

    // A missing Snapshot is rejected at Protocol creation.
    assert!(
        create(vec![
            contexts[0].clone(),
            CrossMarketValidationContextRequest {
                snapshot_id: "missing-snapshot".into(),
                run_override: None,
            },
        ])
        .is_err()
    );
    // Duplicate Snapshots are rejected.
    assert!(create(vec![contexts[0].clone(), contexts[0].clone()]).is_err());
    // A duplicate Instrument context (same market twice) is rejected.
    let btc_again = harness
        .state
        .persist_snapshot_for_user(
            "alice",
            &BarSeries {
                src: "okx".into(),
                code: "BTC-USDT".into(),
                interval: BarInterval::OneHour,
                bars: bars.clone(),
                gaps: vec![],
            },
        )
        .unwrap();
    assert!(
        create(vec![
            contexts[0].clone(),
            CrossMarketValidationContextRequest {
                snapshot_id: btc_again.snapshot_id.clone(),
                run_override: None,
            },
        ])
        .is_err()
    );
    // Incompatible Bar Intervals are rejected.
    let daily_snapshot = harness
        .state
        .persist_snapshot_for_user(
            "alice",
            &BarSeries {
                src: "okx".into(),
                code: "SOL-USDT".into(),
                interval: BarInterval::OneDay,
                bars,
                gaps: vec![],
            },
        )
        .unwrap();
    assert!(
        create(vec![
            contexts[0].clone(),
            CrossMarketValidationContextRequest {
                snapshot_id: daily_snapshot.snapshot_id,
                run_override: None,
            },
        ])
        .is_err()
    );

    let protocol = create(contexts).unwrap();
    let report = harness
        .state
        .validation
        .run_report("alice", &protocol.protocol_id)
        .unwrap();
    assert_eq!(report.cross_market.len(), 2);
    assert_eq!(report.cross_market[0].snapshot.code, "BTC-USDT");
    assert_eq!(report.cross_market[1].snapshot.code, "ETH-USDT");
    assert_eq!(report.aggregate.completed_windows, 2);
    assert_eq!(report.aggregate.failed_windows, 0);
    assert_eq!(
        report
            .cross_market_evidence
            .as_ref()
            .unwrap()
            .completed_markets,
        2
    );
    assert!(report.recommended_contexts.iter().all(|context| {
        context.supporting_report_id == report.report_id
            && context.run.snapshot_id == context.snapshot.snapshot_id
    }));
    // Re-running reuses the exact Backtest Runs and Report identity.
    let run_count = backtest_run_count(&harness.state, "alice");
    let resumed = harness
        .state
        .validation
        .run_report("alice", &protocol.protocol_id)
        .unwrap();
    assert_eq!(resumed.report_id, report.report_id);
    assert_eq!(backtest_run_count(&harness.state, "alice"), run_count);
    assert!(
        harness
            .state
            .validation
            .list_reports("alice")
            .unwrap()
            .iter()
            .any(|listed| listed.report_id == report.report_id)
    );
    assert!(
        harness
            .state
            .validation
            .export_report("alice", &report.report_id, "markdown")
            .unwrap()
            .contains("ETH-USDT")
    );

    // A failing override keeps the study immutable and records the failure.
    let mut failing_override = run_request(&harness);
    failing_override.snapshot_id = eth_snapshot.snapshot_id.clone();
    failing_override.factor_instances.clear();
    let failed_protocol = create(vec![
        CrossMarketValidationContextRequest {
            snapshot_id: harness.snapshot.snapshot_id.clone(),
            run_override: None,
        },
        CrossMarketValidationContextRequest {
            snapshot_id: eth_snapshot.snapshot_id.clone(),
            run_override: Some(failing_override),
        },
    ])
    .unwrap();
    let failed = harness
        .state
        .validation
        .run_report("alice", &failed_protocol.protocol_id)
        .unwrap();
    assert_eq!(failed.aggregate.failed_windows, 1);
    assert!(failed.cross_market[1].failure.is_some());
    assert_eq!(failed.recommended_contexts.len(), 1);

    finish(harness);
}

#[test]
fn reports_list_and_export_are_user_scoped() {
    let harness = harness("report-export");
    let protocol = harness
        .state
        .validation
        .create_protocol(holdout_request(&harness))
        .unwrap();
    let report = harness
        .state
        .validation
        .run_report("alice", &protocol.protocol_id)
        .unwrap();

    assert_eq!(report.windows.len(), 1);
    assert_ne!(
        report.windows[0].sample_in_snapshot_id,
        report.windows[0].sample_out_snapshot_id
    );
    assert_eq!(
        harness
            .state
            .validation
            .list_reports("alice")
            .unwrap()
            .len(),
        1
    );
    assert!(
        harness
            .state
            .validation
            .list_reports("bob")
            .unwrap()
            .is_empty()
    );
    assert!(
        harness
            .state
            .validation
            .export_report("bob", &report.report_id, "json")
            .is_err()
    );
    assert!(
        harness
            .state
            .validation
            .export_report("alice", "missing-report", "json")
            .is_err()
    );
    assert!(
        harness
            .state
            .validation
            .export_report("alice", &report.report_id, "pdf")
            .is_err()
    );

    let json = harness
        .state
        .validation
        .export_report("alice", &report.report_id, "json")
        .unwrap();
    let round_trip: super::ValidationReport = serde_json::from_str(&json).unwrap();
    assert_eq!(round_trip.report_id, report.report_id);
    let markdown = harness
        .state
        .validation
        .export_report("alice", &report.report_id, "markdown")
        .unwrap();
    assert!(markdown.contains(&report.report_id));
    assert!(markdown.contains("research-metrics.md"));

    finish(harness);
}

#[test]
fn validation_evidence_blocks_reset_and_is_user_scoped_in_summary() {
    let harness = harness("reset-hooks");
    let protocol = harness
        .state
        .validation
        .create_protocol(holdout_request(&harness))
        .unwrap();
    let report = harness
        .state
        .validation
        .run_report("alice", &protocol.protocol_id)
        .unwrap();

    let summary = serde_json::to_value(harness.state.local_data_summary("alice").unwrap()).unwrap();
    assert_eq!(summary["protocolCount"], 1);
    assert_eq!(summary["reportCount"], 1);
    assert!(summary["marketDataBlockingRecordCount"].as_u64().unwrap() > 0);
    let bob_summary =
        serde_json::to_value(harness.state.local_data_summary("bob").unwrap()).unwrap();
    assert_eq!(bob_summary["reportCount"], 0);

    // Immutable Validation Reports block Market Data resets.
    let error = harness
        .state
        .reset_local_data("alice", LocalDataResetKind::MarketData)
        .unwrap_err();
    assert!(error.contains("immutable research record"));

    // The run-reference hook protects Backtest Run deletion.
    assert!(
        harness
            .state
            .validation
            .references_run(
                "alice",
                report.windows[0].sample_in_run_id.as_deref().unwrap()
            )
            .unwrap()
    );
    assert!(
        !harness
            .state
            .validation
            .references_run("alice", "unknown-run")
            .unwrap()
    );

    // Reset All removes the Validation evidence for one User only.
    harness
        .state
        .reset_local_data("alice", LocalDataResetKind::All)
        .unwrap();
    let summary = serde_json::to_value(harness.state.local_data_summary("alice").unwrap()).unwrap();
    assert_eq!(summary["protocolCount"], 0);
    assert_eq!(summary["reportCount"], 0);
    assert!(
        harness
            .state
            .validation
            .list_reports("alice")
            .unwrap()
            .is_empty()
    );

    finish(harness);
}

#[test]
fn cross_market_evidence_preserves_order_failures_dispersion_and_report_identity() {
    use super::runner::{aggregate_cross_market, cross_market_evidence, validation_report_id};
    use super::{
        CrossMarketValidationReport, RecommendedContext, ValidationReport, validation_markdown,
    };

    let snapshot = |id: &str, code: &str| MarketDataSnapshot {
        snapshot_id: id.into(),
        src: "okx".into(),
        code: code.into(),
        interval: BarInterval::OneHour,
        start_time_ms: 0,
        end_time_ms: 3_600_000,
        bar_count: 1,
        gaps: vec![],
        parquet_path: PathBuf::new(),
        provenance: None,
        publication_evidence_name: None,
    };
    let run = |snapshot_id: &str| BacktestRunRequest {
        user_id: "alice".into(),
        snapshot_id: snapshot_id.into(),
        portfolio_universe_snapshot_id: None,
        run_start_time_ms: None,
        run_end_time_ms: None,
        factor_instances: vec![],
        signal_instances: vec![],
        strategy_archive_sha256: "strategy".into(),
        strategy_parameters: Default::default(),
        initial_quote_allocation: 1.into(),
        execution_profile: ExecutionProfile {
            maker_fee_rate: Decimal::ZERO,
            taker_fee_rate: Decimal::ZERO,
            adverse_slippage_rate: Decimal::ZERO,
            rebalance_threshold: Decimal::ZERO,
            price_increment: Decimal::ONE,
            quantity_increment: Decimal::ONE,
            minimum_quantity: Decimal::ZERO,
            risk_free_rate: Decimal::ZERO,
            fill_policy: adaq_backtest_core::FillPolicy::Taker,
        },
        strategy_binding: None,
        risk_policy: None,
        seed: 0,
    };
    let metrics = |total_return| adaq_backtest_core::BacktestMetrics {
        initial_equity: 1.into(),
        final_equity: 1.into(),
        total_return,
        cagr: 0.into(),
        annualized_volatility: 0.into(),
        sharpe: 0.into(),
        sortino: 0.into(),
        max_drawdown: 0.into(),
        calmar: 0.into(),
        realized_pnl: 0.into(),
        unrealized_pnl: 0.into(),
        total_fees: 0.into(),
        turnover: 0.into(),
        fill_count: 0,
        realized_trade_count: 0,
        win_rate: 0.into(),
        profit_factor: 0.into(),
        average_win: 0.into(),
        average_loss: 0.into(),
        exposure_time: 0.into(),
        benchmark_return: 0.into(),
        excess_return: 0.into(),
    };
    let contexts = vec![
        CrossMarketValidationReport {
            snapshot: snapshot("btc", "BTC-USDT"),
            run: run("btc"),
            run_id: Some("run-btc".into()),
            metrics: Some(metrics(Decimal::new(20, 2))),
            pauses: vec![],
            failure: None,
        },
        CrossMarketValidationReport {
            snapshot: snapshot("eth", "ETH-USDT"),
            run: run("eth"),
            run_id: None,
            metrics: None,
            pauses: vec![],
            failure: Some("missing-input".into()),
        },
        CrossMarketValidationReport {
            snapshot: snapshot("sol", "SOL-USDT"),
            run: run("sol"),
            run_id: Some("run-sol".into()),
            metrics: Some(metrics(Decimal::new(-10, 2))),
            pauses: vec![],
            failure: None,
        },
    ];
    let aggregate = aggregate_cross_market(&contexts);
    assert_eq!(aggregate.completed_windows, 2);
    assert_eq!(aggregate.failed_windows, 1);
    assert_eq!(
        cross_market_evidence(&contexts)
            .unwrap()
            .total_return_spread,
        Decimal::new(30, 2)
    );

    let report = ValidationReport {
        report_id: String::new(),
        protocol_id: "protocol".into(),
        user_id: "alice".into(),
        method_version: "cross-market@1".into(),
        aggregation_rule_version: "equal-window@1".into(),
        strategy_binding: None,
        final_evidence_sealed: false,
        walk_forward: None,
        cross_market: contexts.clone(),
        windows: vec![],
        aggregate,
        recommended_contexts: vec![RecommendedContext {
            supporting_report_id: String::new(),
            snapshot: contexts[0].snapshot.clone(),
            run: contexts[0].run.clone(),
        }],
        cross_market_evidence: cross_market_evidence(&contexts),
    };
    let identity = validation_report_id(&report).unwrap();
    let exported = serde_json::to_value(&report).unwrap();
    assert_eq!(
        exported["recommendedContexts"][0]["supportingReportId"],
        serde_json::Value::String(String::new())
    );
    assert!(validation_markdown(&report).contains("crossMarketEvidence"));
    assert!(validation_markdown(&report).contains("recommendedContexts"));
    let mut reordered = report;
    reordered.cross_market.reverse();
    assert_ne!(identity, validation_report_id(&reordered).unwrap());
}
