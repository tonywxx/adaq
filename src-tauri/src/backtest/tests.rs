//! Interface-level Backtest Run tests (real SQLite + temp directories).
//!
//! These tests exercise the module through its public interface —
//! preflight, execution, Run reuse, chart and execution data, listing,
//! deletion, the component-lock query, and the summary and reset hooks —
//! against a test-fake Source adapter. Where a test must seed Runs or
//! observe bridge rows it does so through the module's own database
//! handle. The production composition-root adapter is covered by the
//! composition-root, Validation, and forecast_evaluation interface tests.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use adaq_backtest_core::{
    EvaluationWindow, ExecutionProfile, MarketDataSnapshot, StrategyBinding, StrategyProject,
    StrategyScope, StrategyWindow,
};
use adaq_component_tooling::{ComponentManifest, ComponentPackage, pack_component};
use adaq_data_core::{BarInterval, OhlcvBar};
use rusqlite::{Connection, params};

use super::{
    BacktestChartRequest, BacktestExecutionRequest, BacktestListRequest, BacktestRunRequest,
    BacktestSource, Backtests, ComponentPackageSource, SnapshotReadSource,
};

struct FakeBacktestSource {
    database: Arc<Mutex<Connection>>,
    packages: Mutex<HashMap<(String, String), ComponentPackage>>,
    snapshots: Mutex<HashMap<(String, String), (MarketDataSnapshot, Vec<OhlcvBar>)>>,
    runtime_dir: PathBuf,
    referenced_runs: Mutex<HashSet<(String, String)>>,
}

impl SnapshotReadSource for FakeBacktestSource {
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.snapshots
            .lock()
            .unwrap()
            .get(&(user_id.to_owned(), snapshot_id.to_owned()))
            .cloned()
            .ok_or_else(|| "Market Data Snapshot is not available to this User".to_owned())
    }
}

impl ComponentPackageSource for FakeBacktestSource {
    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        self.packages
            .lock()
            .unwrap()
            .get(&(user_id.to_owned(), archive_sha256.to_owned()))
            .cloned()
            .ok_or_else(|| "Component Package is not available to this User".to_owned())
    }

    fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.runtime_dir).map_err(|error| error.to_string())?;
        let path = self
            .runtime_dir
            .join(format!("{}.wasm", package.manifest.wasm_sha256));
        if !path.is_file() {
            fs::write(&path, &package.wasm).map_err(|error| error.to_string())?;
        }
        Ok(path)
    }
}

impl BacktestSource for FakeBacktestSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(|error| error.to_string())
    }

    fn signal_datasets(
        &self,
        _user_id: &str,
        _include_rows: bool,
        _dataset_ids: Option<&[String]>,
    ) -> Result<Vec<crate::forecast_signal_dataset::BacktestSignalDataset>, String> {
        Ok(vec![])
    }

    fn validation_report_references_run(
        &self,
        user_id: &str,
        run_id: &str,
    ) -> Result<bool, String> {
        Ok(self
            .referenced_runs
            .lock()
            .unwrap()
            .contains(&(user_id.to_owned(), run_id.to_owned())))
    }
}

struct Harness {
    root: PathBuf,
    module: Backtests,
    source: Arc<FakeBacktestSource>,
    strategy_hash: String,
    snapshot_id: String,
}

fn fixture(name: &str) -> (ComponentManifest, Vec<u8>) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    let manifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let wasm = fs::read(directory.join(format!(
        "target/wasm32-unknown-unknown/debug/m1_{name}_fixture.wasm"
    )))
    .unwrap();
    (manifest, wasm)
}

/// Builds a deterministic fixture Backtest environment on a real SQLite
/// file: one 3-bar hourly BTC-USDT Snapshot entitled to alice, and the
/// fixture Strategy Component entitled to alice.
fn harness(name: &str) -> Harness {
    let root = std::env::temp_dir().join(format!(
        "adaq-backtest-module-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let database = Connection::open(root.join("adaq.db")).unwrap();
    // The referenced tables the bridge FKs point at; the Component Library
    // and forecast_signal_dataset domains own their real schema.
    database
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS component_content (
                archive_sha256 TEXT PRIMARY KEY,
                component_id TEXT NOT NULL,
                version TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                wasm_sha256 TEXT NOT NULL,
                archive_path TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT ''
             );
             CREATE TABLE IF NOT EXISTS signal_dataset_content (
                dataset_id TEXT PRIMARY KEY,
                metadata_json TEXT NOT NULL,
                parquet_path TEXT NOT NULL
             );",
        )
        .unwrap();
    let (manifest, wasm) = fixture("strategy");
    let bytes = pack_component(manifest, &wasm).unwrap();
    let package = ComponentPackage::read(&bytes).unwrap();
    let strategy_hash = package.archive_sha256.clone();
    database
        .execute(
            "INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path)
             VALUES (?1, ?2, ?3, ?4, 'strategy', ?5, ?6)",
            params![
                package.archive_sha256,
                package.manifest.component_id.to_string(),
                package.manifest.version.to_string(),
                package.manifest.name,
                package.manifest.wasm_sha256,
                root.join("strategy.adaq").to_string_lossy(),
            ],
        )
        .unwrap();
    let snapshot_id = "snapshot-1".to_owned();
    let bar = |open_time_ms: i64, close: i64| OhlcvBar {
        open_time_ms,
        open: close.into(),
        high: close.into(),
        low: close.into(),
        close: close.into(),
        base_volume: rust_decimal::Decimal::ONE,
        quote_volume: close.into(),
    };
    let snapshot = MarketDataSnapshot {
        snapshot_id: snapshot_id.clone(),
        src: "okx".into(),
        code: "BTC-USDT".into(),
        interval: BarInterval::OneHour,
        start_time_ms: 0,
        end_time_ms: 7_200_000,
        bar_count: 3,
        gaps: vec![],
        parquet_path: PathBuf::new(),
        provenance: None,
    };
    let source = Arc::new(FakeBacktestSource {
        database: Arc::new(Mutex::new(database)),
        packages: Mutex::new(HashMap::from([(
            ("alice".to_owned(), strategy_hash.clone()),
            package,
        )])),
        snapshots: Mutex::new(HashMap::from([(
            ("alice".to_owned(), snapshot_id.clone()),
            (
                snapshot,
                vec![bar(0, 100), bar(3_600_000, 101), bar(7_200_000, 102)],
            ),
        )])),
        runtime_dir: root.join("runtime"),
        referenced_runs: Mutex::new(HashSet::new()),
    });
    let module = Backtests::open(source.clone()).unwrap();
    Harness {
        root,
        module,
        source,
        strategy_hash,
        snapshot_id,
    }
}

fn finish(harness: Harness) {
    drop(harness.module);
    drop(harness.source);
    fs::remove_dir_all(harness.root).unwrap();
}

fn run_request(harness: &Harness) -> BacktestRunRequest {
    BacktestRunRequest {
        user_id: "alice".into(),
        snapshot_id: harness.snapshot_id.clone(),
        run_start_time_ms: None,
        run_end_time_ms: None,
        factor_instances: vec![],
        signal_instances: vec![],
        strategy_archive_sha256: harness.strategy_hash.clone(),
        strategy_parameters: HashMap::new(),
        initial_quote_allocation: 10_000.into(),
        execution_profile: ExecutionProfile {
            maker_fee_rate: rust_decimal::Decimal::new(8, 4),
            taker_fee_rate: rust_decimal::Decimal::new(1, 3),
            adverse_slippage_rate: rust_decimal::Decimal::ZERO,
            rebalance_threshold: rust_decimal::Decimal::ZERO,
            price_increment: rust_decimal::Decimal::ONE,
            quantity_increment: rust_decimal::Decimal::new(1, 4),
            minimum_quantity: rust_decimal::Decimal::new(1, 4),
            risk_free_rate: rust_decimal::Decimal::ZERO,
            fill_policy: adaq_backtest_core::FillPolicy::Taker,
        },
        seed: 0,
    }
}

#[test]
fn pipeline_preflights_executes_reuses_and_scopes_runs_by_user() {
    let harness = harness("vertical");

    let preview = harness.module.preflight(&run_request(&harness)).unwrap();
    assert!(!preview.reuses_existing_run);
    assert!(preview.feature_plan.get("slots").is_some());
    assert_eq!(
        preview.normalized_request.initial_quote_allocation,
        rust_decimal::Decimal::from(10_000),
    );
    assert_eq!(preview.component_lock.len(), 1);
    assert_eq!(
        preview.component_lock[0].archive_sha256,
        harness.strategy_hash
    );
    // A window outside the exact Snapshot is rejected before execution.
    let mut subset = run_request(&harness);
    subset.run_start_time_ms = Some(-1);
    assert!(
        harness
            .module
            .preflight(&subset)
            .err()
            .unwrap()
            .contains("subset")
    );

    let first = harness.module.run(run_request(&harness)).unwrap();
    assert!(!first.plan_hash.is_empty());
    let replay = harness.module.run(run_request(&harness)).unwrap();
    assert_eq!(first.run_id, replay.run_id);
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&replay).unwrap()
    );
    assert!(
        harness
            .module
            .preflight(&run_request(&harness))
            .unwrap()
            .reuses_existing_run
    );
    assert_eq!(
        harness
            .module
            .get("alice", &first.run_id)
            .unwrap()
            .provenance,
        first.provenance
    );
    // The seed participates in the deterministic Run identity.
    let mut changed = run_request(&harness);
    changed.seed = 1;
    let changed = harness.module.run(changed).unwrap();
    assert_ne!(first.run_id, changed.run_id);

    // Runs are user-scoped.
    assert!(harness.module.get("bob", &first.run_id).is_err());
    assert_eq!(
        harness
            .module
            .list(&BacktestListRequest {
                user_id: "bob".into(),
                src: None,
                code: None,
                page: 1,
            })
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        harness
            .module
            .list(&BacktestListRequest {
                user_id: "alice".into(),
                src: None,
                code: None,
                page: 1,
            })
            .unwrap()
            .total,
        2
    );

    // Chart data validates its range and aggregates within it.
    let chart = harness
        .module
        .chart_data(&BacktestChartRequest {
            user_id: "alice".into(),
            run_id: first.run_id.clone(),
            start_time_ms: 0,
            end_time_ms: 7_200_000,
            max_points: 100,
        })
        .unwrap();
    assert!(!chart.bars.is_empty());
    assert!(
        harness
            .module
            .chart_data(&BacktestChartRequest {
                user_id: "alice".into(),
                run_id: first.run_id.clone(),
                start_time_ms: 7_200_000,
                end_time_ms: 0,
                max_points: 100,
            })
            .is_err()
    );

    // Execution data pages orders and fills.
    let page = harness
        .module
        .execution_data(&BacktestExecutionRequest {
            user_id: "alice".into(),
            run_id: first.run_id.clone(),
            offset: 0,
            limit: 1,
        })
        .unwrap();
    assert!(page.orders.len() <= 1);
    assert!(page.fills.len() <= 1);
    assert_eq!(page.total_orders, first.result.orders.len());
    assert_eq!(page.total_fills, first.result.fills.len());
    assert!(
        harness
            .module
            .execution_data(&BacktestExecutionRequest {
                user_id: "alice".into(),
                run_id: first.run_id.clone(),
                offset: 0,
                limit: 0,
            })
            .is_err()
    );

    // The component-lock query reports the Strategy locked by both Runs in
    // creation order.
    let database = harness.module.0.database().unwrap();
    let locks = harness
        .module
        .runs_locking_components(&database, "alice")
        .unwrap();
    assert_eq!(
        locks.get(&harness.strategy_hash).unwrap(),
        &[first.run_id.clone(), changed.run_id.clone()]
    );
    assert!(
        harness
            .module
            .runs_locking_components(&database, "bob")
            .unwrap()
            .is_empty()
    );
    drop(database);

    // An immutable Validation Report reference blocks deletion.
    harness
        .source
        .referenced_runs
        .lock()
        .unwrap()
        .insert(("alice".to_owned(), first.run_id.clone()));
    assert_eq!(
        harness.module.delete("alice", &first.run_id).unwrap_err(),
        "Backtest Run is referenced by an immutable Validation Report"
    );
    harness
        .source
        .referenced_runs
        .lock()
        .unwrap()
        .remove(&("alice".to_owned(), first.run_id.clone()));

    harness.module.delete("alice", &first.run_id).unwrap();
    assert!(harness.module.get("alice", &first.run_id).is_err());
    assert_eq!(
        harness.module.delete("alice", &first.run_id).unwrap_err(),
        "Backtest Run was not found"
    );
    // Bridge rows cascade with the deleted Run.
    let bridge_count: i64 = harness
        .module
        .0
        .database()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM backtest_run_components WHERE run_id = ?1",
            [&first.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bridge_count, 0);

    finish(harness);
}

#[test]
fn stored_provenance_is_validated_and_optional() {
    let harness = harness("provenance");
    let first = harness.module.run(run_request(&harness)).unwrap();

    // Legacy Runs recorded before provenance existed load unchanged.
    let stored = harness.module.load_run("alice", &first.run_id).unwrap();
    let mut legacy_json = serde_json::to_value(&stored).unwrap();
    legacy_json.as_object_mut().unwrap().remove("provenance");
    harness
        .module
        .0
        .database()
        .unwrap()
        .execute(
            "UPDATE backtest_runs SET result_json = ?1 WHERE run_id = ?2",
            params![legacy_json.to_string(), first.run_id],
        )
        .unwrap();
    assert!(
        harness
            .module
            .get("alice", &first.run_id)
            .unwrap()
            .provenance
            .is_none()
    );

    // A tampered Component Lock is rejected on read.
    let mut tampered = serde_json::to_value(&stored).unwrap();
    tampered["provenance"]["componentLock"][0]["archiveSha256"] = serde_json::json!("f".repeat(64));
    harness
        .module
        .0
        .database()
        .unwrap()
        .execute(
            "UPDATE backtest_runs SET result_json = ?1 WHERE run_id = ?2",
            params![tampered.to_string(), first.run_id],
        )
        .unwrap();
    assert!(harness.module.get("alice", &first.run_id).is_err());

    finish(harness);
}

fn seed_run(
    harness: &Harness,
    run_id: &str,
    user_id: &str,
    created_at: &str,
    code: &str,
    locked_hashes: &[&str],
) {
    let database = harness.module.0.database().unwrap();
    for hash in locked_hashes {
        database
            .execute(
                "INSERT OR IGNORE INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path)
                 VALUES (?1, 'component', '1.0.0', 'Locked', 'factor', 'wasm', ?2)",
                params![hash, harness.root.join(format!("{hash}.adaq")).to_string_lossy()],
            )
            .unwrap();
    }
    let json = serde_json::json!({
        "snapshot": {
            "snapshotId": format!("{user_id}-{code}"),
            "src": "okx",
            "code": code,
            "interval": "1h",
            "barCount": 100
        },
        "result": { "metrics": { "totalReturn": "0.1" } }
    });
    database
        .execute(
            "INSERT INTO backtest_runs(run_id, user_id, created_at, result_json) VALUES (?1, ?2, ?3, ?4)",
            params![run_id, user_id, created_at, json.to_string()],
        )
        .unwrap();
    for hash in locked_hashes {
        database
            .execute(
                "INSERT INTO backtest_run_components VALUES (?1, ?2)",
                params![run_id, hash],
            )
            .unwrap();
    }
}

#[test]
fn run_history_is_filtered_and_paged_by_instrument() {
    let harness = harness("history");
    for index in 0..12 {
        seed_run(
            &harness,
            &format!("btc-{index}"),
            "alice",
            &format!("2026-07-30 00:{index:02}:00"),
            "BTC-USDT",
            &[],
        );
    }
    seed_run(
        &harness,
        "alice-ETH-USDT",
        "alice",
        "2026-07-30 01:00:00",
        "ETH-USDT",
        &[],
    );
    seed_run(
        &harness,
        "bob-BTC-USDT",
        "bob",
        "2026-07-30 01:00:00",
        "BTC-USDT",
        &[],
    );

    let first = harness
        .module
        .list(&BacktestListRequest {
            user_id: "alice".into(),
            src: Some("okx".into()),
            code: Some("BTC-USDT".into()),
            page: 1,
        })
        .unwrap();
    assert_eq!(first.total, 12);
    assert_eq!(first.items.len(), 10);
    assert!(first.items.iter().all(|run| run.code == "BTC-USDT"));

    let second = harness
        .module
        .list(&BacktestListRequest {
            user_id: "alice".into(),
            src: Some("okx".into()),
            code: Some("BTC-USDT".into()),
            page: 2,
        })
        .unwrap();
    assert_eq!(second.total, 12);
    assert_eq!(second.items.len(), 2);

    let eth = harness
        .module
        .list(&BacktestListRequest {
            user_id: "alice".into(),
            src: Some("okx".into()),
            code: Some("ETH-USDT".into()),
            page: 1,
        })
        .unwrap();
    assert_eq!(eth.total, 1);
    assert_eq!(eth.items[0].code, "ETH-USDT");

    let all = harness
        .module
        .list(&BacktestListRequest {
            user_id: "alice".into(),
            src: None,
            code: None,
            page: 1,
        })
        .unwrap();
    assert_eq!(all.total, 13);
    assert_eq!(all.items.len(), 10);
    assert!(
        harness
            .module
            .list(&BacktestListRequest {
                user_id: "alice".into(),
                src: Some("okx".into()),
                code: None,
                page: 1,
            })
            .is_err()
    );

    finish(harness);
}

#[test]
fn summary_reset_and_orphan_guard_hooks_are_user_scoped() {
    let harness = harness("hooks");
    let locked_hash = "a".repeat(64);
    let alice_only_hash = "b".repeat(64);
    seed_run(
        &harness,
        "run-1",
        "alice",
        "2026-07-30 00:00:00",
        "BTC-USDT",
        &[&locked_hash],
    );
    seed_run(
        &harness,
        "run-2",
        "alice",
        "2026-07-30 00:01:00",
        "BTC-USDT",
        &[&locked_hash, &alice_only_hash],
    );
    seed_run(
        &harness,
        "run-3",
        "bob",
        "2026-07-30 00:02:00",
        "BTC-USDT",
        &[&locked_hash],
    );

    assert_eq!(
        harness.module.summary_for_user("alice").unwrap().run_count,
        2
    );
    assert_eq!(harness.module.summary_for_user("bob").unwrap().run_count, 1);

    // The lock query orders Runs by creation for both deletion-lock and
    // listing-lock consumers, and the orphan guard reports every
    // Run-locked hash, or only other Users' locks when the reset User is
    // excluded.
    {
        let database = harness.module.0.database().unwrap();
        let locks = harness
            .module
            .runs_locking_components(&database, "alice")
            .unwrap();
        assert_eq!(
            locks[&locked_hash],
            vec!["run-1".to_owned(), "run-2".to_owned()]
        );
        assert_eq!(locks[&alice_only_hash], vec!["run-2".to_owned()]);
        let all = harness
            .module
            .component_hashes_locked_by_runs(&database, None)
            .unwrap();
        assert_eq!(
            all,
            HashSet::from([locked_hash.clone(), alice_only_hash.clone()])
        );
        let excluding_alice = harness
            .module
            .component_hashes_locked_by_runs(&database, Some("alice"))
            .unwrap();
        assert_eq!(excluding_alice, HashSet::from([locked_hash.clone()]));
        let excluding_bob = harness
            .module
            .component_hashes_locked_by_runs(&database, Some("bob"))
            .unwrap();
        assert_eq!(
            excluding_bob,
            HashSet::from([locked_hash.clone(), alice_only_hash.clone()])
        );
    }

    // Reset drops only the reset User's Runs; bridge rows cascade.
    {
        let mut database = harness.module.0.database().unwrap();
        let transaction = database.transaction().unwrap();
        harness
            .module
            .reset_for_user(&transaction, "alice")
            .unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(
        harness.module.summary_for_user("alice").unwrap().run_count,
        0
    );
    assert_eq!(harness.module.summary_for_user("bob").unwrap().run_count, 1);
    assert!(
        harness
            .module
            .runs_locking_components(&harness.module.0.database().unwrap(), "alice")
            .unwrap()
            .is_empty()
    );
    let remaining: Vec<String> = harness
        .module
        .0
        .database()
        .unwrap()
        .prepare("SELECT run_id FROM backtest_run_components ORDER BY run_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(remaining, vec!["run-3".to_owned()]);

    finish(harness);
}

#[test]
fn incompatible_stored_feature_plan_fails_with_reset_required_diagnostic() {
    let provenance = super::BacktestRunProvenance {
        normalized_request: super::NormalizedBacktestRunRequest {
            snapshot_id: "snapshot-1".into(),
            run_start_time_ms: None,
            run_end_time_ms: None,
            strategy_archive_sha256: "a".repeat(64),
            strategy_parameters: std::collections::BTreeMap::new(),
            factor_instances: Vec::new(),
            signal_instances: Vec::new(),
            initial_quote_allocation: rust_decimal::Decimal::ONE,
            execution_profile: ExecutionProfile {
                maker_fee_rate: rust_decimal::Decimal::new(8, 4),
                taker_fee_rate: rust_decimal::Decimal::new(1, 3),
                adverse_slippage_rate: rust_decimal::Decimal::ZERO,
                rebalance_threshold: rust_decimal::Decimal::ZERO,
                price_increment: rust_decimal::Decimal::ONE,
                quantity_increment: rust_decimal::Decimal::new(1, 4),
                minimum_quantity: rust_decimal::Decimal::new(1, 4),
                risk_free_rate: rust_decimal::Decimal::ZERO,
                fill_policy: adaq_backtest_core::FillPolicy::Taker,
            },
            seed: 1,
        },
        feature_plan_json: r#"{"planSchemaVersion":"1.0.0"}"#.into(),
        feature_plan_hash: "b".repeat(64),
        component_lock: Vec::new(),
        dataset_lock: Vec::new(),
        architecture: adaq_component_tooling::StrategyArchitecture::Composed,
        indicator_engine_build_identity: super::IndicatorEngineBuildIdentity {
            engine_version: "test".into(),
            ta_lib_version: "test".into(),
            ta_source_sha256: "c".repeat(64),
            catalog_version: "test".into(),
            wrapper_sha256: "d".repeat(64),
            target_triple: "test".into(),
            compiler_and_flags_sha256: "e".repeat(64),
            engine_build_id: "test".into(),
        },
        backtest_engine_version: "test".into(),
        seed: 1,
    };
    let error = super::pipeline::validate_provenance(&provenance).unwrap_err();
    assert!(error.contains("reset-required"), "{error}");
    assert!(
        error.contains("incompatible Feature Plan schema"),
        "{error}"
    );

    let mut invalid = provenance;
    invalid.feature_plan_json = "{".into();
    let error = super::pipeline::validate_provenance(&invalid).unwrap_err();
    assert!(error.contains("invalid frozen Feature Plan"), "{error}");
}

#[test]
fn strategy_projects_are_user_scoped_append_only_and_retain_attempt_evidence() {
    let harness = harness("strategy");
    let project = StrategyProject::create(
        "strategy-project",
        "alice",
        harness.strategy_hash.clone(),
        StrategyScope::Portfolio,
        "context-1",
        0,
        100,
        StrategyWindow {
            start_time_ms: 1,
            end_time_ms: 40,
        },
        StrategyWindow {
            start_time_ms: 60,
            end_time_ms: 90,
        },
        vec![StrategyBinding {
            slot: "forecast".into(),
            evidence_id: "dataset-1".into(),
            lineage_hash: "lineage-1".into(),
        }],
        Default::default(),
    )
    .unwrap();
    harness.module.save_strategy_project(&project).unwrap();
    assert_eq!(harness.module.strategy_projects("alice").unwrap().len(), 1);
    assert!(harness.module.strategy_projects("bob").unwrap().is_empty());

    let attempt = harness
        .module
        .start_strategy_attempt("alice", "strategy-project", EvaluationWindow::Final)
        .unwrap();
    let completed = harness
        .module
        .complete_strategy_attempt("alice", &attempt.attempt_id, "run-1")
        .unwrap();
    assert_eq!(
        completed.status,
        adaq_backtest_core::StrategyAttemptStatus::Completed
    );
    assert_eq!(completed.evidence.unwrap().run_ids, vec!["run-1"]);
    finish(harness);
}
