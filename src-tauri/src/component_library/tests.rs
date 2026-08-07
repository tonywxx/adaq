//! Interface-level tests for the Component Library module: a real SQLite
//! file, temp directories, real Component Packages, and the real Backtest
//! Run module composed as the lock source — the same composition the
//! composition root builds, minus the unrelated domains.

use std::{
    io::{Cursor, Write},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use adaq_backtest_core::{ExecutionProfile, MarketDataSnapshot};
use adaq_component_tooling::{ComponentManifest, pack_component};
use adaq_data_core::{BarInterval, OhlcvBar};
use rusqlite::{Connection, params};
use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;
use crate::backtest::{
    BacktestRunRequest, BacktestSource, Backtests, ComponentPackageSource, SnapshotReadSource,
};

/// The test lock source binds the real Backtest Run module after both
/// modules are open, mirroring the composition root's post-construction
/// binding.
struct HarnessSource {
    database: Arc<Mutex<Connection>>,
    archive_directory: PathBuf,
    backtests: Mutex<Option<Backtests>>,
}

impl ComponentLockSource for HarnessSource {
    fn runs_locking_components(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        self.backtests
            .lock()
            .unwrap()
            .as_ref()
            .expect("backtest module is bound")
            .runs_locking_components(database, user_id)
    }

    fn component_hashes_locked_by_runs(
        &self,
        database: &Connection,
        excluding_user: Option<&str>,
    ) -> Result<HashSet<String>, String> {
        self.backtests
            .lock()
            .unwrap()
            .as_ref()
            .expect("backtest module is bound")
            .component_hashes_locked_by_runs(database, excluding_user)
    }
}

impl ComponentSource for HarnessSource {
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn archive_directory(&self) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.archive_directory).map_err(string)?;
        Ok(self.archive_directory.clone())
    }

    fn locks(&self) -> &dyn ComponentLockSource {
        self
    }
}

/// Backtest Run dependencies resolved against the Component Library
/// itself, so Run execution locks real imported Packages.
struct HarnessBacktestSource {
    database: Arc<Mutex<Connection>>,
    components: ComponentLibrary,
    snapshots: Mutex<HashMap<(String, String), (MarketDataSnapshot, Vec<OhlcvBar>)>>,
    runtime_dir: PathBuf,
}

impl SnapshotReadSource for HarnessBacktestSource {
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

impl ComponentPackageSource for HarnessBacktestSource {
    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        self.components.package_for_user(user_id, archive_sha256)
    }

    fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.runtime_dir).map_err(string)?;
        let path = self
            .runtime_dir
            .join(format!("{}.wasm", package.manifest.wasm_sha256));
        if !path.is_file() {
            fs::write(&path, &package.wasm).map_err(string)?;
        }
        Ok(path)
    }
}

impl BacktestSource for HarnessBacktestSource {
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn signal_datasets(
        &self,
        _user_id: &str,
        _include_rows: bool,
        _dataset_ids: Option<&[String]>,
    ) -> Result<Vec<crate::m8::BacktestSignalDataset>, String> {
        Ok(vec![])
    }

    fn validation_report_references_run(
        &self,
        _user_id: &str,
        _run_id: &str,
    ) -> Result<bool, String> {
        Ok(false)
    }
}

struct Harness {
    root: PathBuf,
    components: ComponentLibrary,
    backtests: Backtests,
    source: Arc<HarnessSource>,
    backtest_source: Arc<HarnessBacktestSource>,
}

fn harness(name: &str) -> Harness {
    let root = std::env::temp_dir().join(format!(
        "adaq-component-library-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let database = Connection::open(root.join("adaq.db")).unwrap();
    // The Signal Dataset tables the deletion dataset-lock check reads; m8
    // owns their real schema.
    database
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS signal_dataset_content (
                dataset_id TEXT PRIMARY KEY,
                metadata_json TEXT NOT NULL,
                parquet_path TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS signal_dataset_access (
                user_id TEXT NOT NULL,
                dataset_id TEXT NOT NULL,
                PRIMARY KEY(user_id, dataset_id),
                FOREIGN KEY(dataset_id) REFERENCES signal_dataset_content(dataset_id)
             );",
        )
        .unwrap();
    let source = Arc::new(HarnessSource {
        database: Arc::new(Mutex::new(database)),
        archive_directory: root.join("components"),
        backtests: Mutex::new(None),
    });
    let components = ComponentLibrary::open(source.clone()).unwrap();
    let backtest_source = Arc::new(HarnessBacktestSource {
        database: source.database.clone(),
        components: components.clone(),
        snapshots: Mutex::new(HashMap::new()),
        runtime_dir: root.join("runtime"),
    });
    let backtests = Backtests::open(backtest_source.clone()).unwrap();
    *source.backtests.lock().unwrap() = Some(backtests.clone());
    Harness {
        root,
        components,
        backtests,
        source,
        backtest_source,
    }
}

fn finish(harness: Harness) {
    drop(harness.components);
    drop(harness.backtests);
    drop(harness.source);
    drop(harness.backtest_source);
    fs::remove_dir_all(harness.root).unwrap();
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

fn legacy_package() -> Vec<u8> {
    let manifest = r#"{
        "componentId":"22222222-2222-4222-8222-222222222222",
        "version":"1.0.0",
        "name":"Legacy Strategy",
        "kind":"strategy",
        "abiVersion":"1.0.0",
        "inputNames":["trend.close-change"]
    }"#;
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    archive.start_file("manifest.json", options).unwrap();
    archive.write_all(manifest.as_bytes()).unwrap();
    archive.start_file("component.wasm", options).unwrap();
    archive.write_all(b"\0asm\x0d\0\x01\0").unwrap();
    archive.finish().unwrap().into_inner()
}

fn seed_snapshot(harness: &Harness, user_id: &str, snapshot_id: &str) {
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
        snapshot_id: snapshot_id.into(),
        src: "okx".into(),
        code: "BTC-USDT".into(),
        interval: BarInterval::OneHour,
        start_time_ms: 0,
        end_time_ms: 7_200_000,
        bar_count: 3,
        gaps: vec![],
        parquet_path: PathBuf::new(),
    };
    harness.backtest_source.snapshots.lock().unwrap().insert(
        (user_id.to_owned(), snapshot_id.to_owned()),
        (
            snapshot,
            vec![bar(0, 100), bar(3_600_000, 101), bar(7_200_000, 102)],
        ),
    );
}

fn run_request(snapshot_id: &str, strategy_hash: &str) -> BacktestRunRequest {
    BacktestRunRequest {
        user_id: "alice".into(),
        snapshot_id: snapshot_id.into(),
        run_start_time_ms: None,
        run_end_time_ms: None,
        factor_instances: vec![],
        signal_instances: vec![],
        strategy_archive_sha256: strategy_hash.into(),
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
fn import_is_user_scoped_and_identity_locked() {
    let harness = harness("import");
    let (factor, wasm) = fixture("factor");
    let bytes = pack_component(factor.clone(), &wasm).unwrap();
    let factor_entry = harness.components.import("alice", &bytes).unwrap();
    assert_eq!(
        factor_entry.manifest_schema_version,
        factor.manifest_schema_version.to_string()
    );
    assert_eq!(factor_entry.abi_version, factor.abi_version.to_string());
    assert_eq!(factor_entry.output_names, factor.output_names);
    assert_eq!(factor_entry.warmup_bars, factor.warmup_bars);
    assert!(factor_entry.compatible);
    assert!(factor_entry.locked_by_run_ids.is_empty());
    assert!(
        harness
            .components
            .is_imported("alice", &factor_entry.archive_sha256)
            .unwrap()
    );
    assert!(
        !harness
            .components
            .is_imported("bob", &factor_entry.archive_sha256)
            .unwrap()
    );
    assert_eq!(harness.components.list("alice").unwrap().len(), 1);
    assert!(harness.components.list("bob").unwrap().is_empty());
    assert_eq!(
        harness
            .components
            .delete("bob", &factor_entry.archive_sha256)
            .unwrap_err(),
        "Component Package is not available to this User"
    );

    // Duplicate detection: reusing an identity and version with different
    // content is rejected; reimporting the identical Package grants the
    // second User access without duplicating content.
    let mut conflicting = factor;
    conflicting.name = "Conflicting Package".into();
    assert_eq!(
        harness
            .components
            .import("alice", &pack_component(conflicting, &wasm).unwrap())
            .unwrap_err(),
        "A different Component already uses this identity and version"
    );
    let reimported = harness.components.import("bob", &bytes).unwrap();
    assert_eq!(reimported.archive_sha256, factor_entry.archive_sha256);
    assert_eq!(harness.components.list("bob").unwrap().len(), 1);

    let (strategy, wasm) = fixture("strategy");
    let strategy_entry = harness
        .components
        .import("alice", &pack_component(strategy, &wasm).unwrap())
        .unwrap();
    assert!(!strategy_entry.feature_slots.is_empty());
    assert_eq!(harness.components.list("alice").unwrap().len(), 2);
    finish(harness);
}

#[test]
fn paging_returns_ten_packages_per_page() {
    let harness = harness("paging");
    let database = harness.source.database.lock().unwrap();
    for index in 0..12 {
        let archive_sha256 = format!("{index:064x}");
        let path = harness.root.join(format!("invalid-{index}.adaq"));
        fs::write(&path, b"invalid package").unwrap();
        database
            .execute(
                "INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path) VALUES (?1, ?2, '1.0.0', ?3, 'factor', ?4, ?5)",
                params![
                    archive_sha256,
                    format!("00000000-0000-4000-8000-{index:012}"),
                    format!("Package {index:02}"),
                    format!("{:064x}", index + 100),
                    path.to_string_lossy()
                ],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO component_access VALUES ('alice', ?1)",
                [archive_sha256],
            )
            .unwrap();
    }
    drop(database);

    let first = harness.components.page("alice", 1).unwrap();
    assert_eq!(first.total, 12);
    assert_eq!(first.items.len(), 10);
    assert!(first.items.iter().all(|item| !item.compatible));

    let second = harness.components.page("alice", 2).unwrap();
    assert_eq!(second.total, 12);
    assert_eq!(second.items.len(), 2);

    assert_eq!(
        harness.components.page("alice", 0).err().unwrap(),
        "Component Package page is invalid"
    );
    finish(harness);
}

#[test]
fn compatible_factors_match_identity_and_version_only() {
    let harness = harness("compatible-factors");
    // An incompatible legacy entry is listed as such and never blocks
    // Factor queries or deletion.
    let archive_hash = "a".repeat(64);
    let path = harness.root.join("legacy.adaq");
    fs::write(&path, legacy_package()).unwrap();
    let database = harness.source.database.lock().unwrap();
    database
        .execute(
            "INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                archive_hash,
                "22222222-2222-4222-8222-222222222222",
                "1.0.0",
                "Legacy Strategy",
                "strategy",
                "b".repeat(64),
                path.to_string_lossy()
            ],
        )
        .unwrap();
    database
        .execute(
            "INSERT INTO component_access VALUES ('alice', ?1)",
            [&archive_hash],
        )
        .unwrap();
    drop(database);

    let components = harness.components.list("alice").unwrap();
    assert_eq!(components.len(), 1);
    assert!(
        components[0]
            .compatibility_error
            .as_deref()
            .unwrap()
            .contains("inputNames")
    );

    let factor = harness
        .components
        .import("alice", &public_example_package("factor-close-momentum-5"))
        .unwrap();
    let strategy = harness
        .components
        .import("alice", &public_example_package("strategy-momentum-trend"))
        .unwrap();
    let matches = harness
        .components
        .compatible_factors("alice", &strategy.archive_sha256)
        .unwrap();
    assert_eq!(matches["momentum"], [factor.archive_sha256.clone()]);
    assert_eq!(
        harness
            .components
            .compatible_factors("alice", &factor.archive_sha256)
            .unwrap_err(),
        "Compatible Factors require a Strategy or Model Component"
    );

    harness.components.delete("alice", &archive_hash).unwrap();
    assert!(
        !harness
            .components
            .is_imported("alice", &archive_hash)
            .unwrap()
    );
    finish(harness);
}

#[test]
fn compatible_factor_hashes_filter_identity_version_and_compatibility() {
    let harness = harness("factor-hash-filtering");
    let factor_bytes = public_example_package("factor-close-momentum-5");
    let strategy_bytes = public_example_package("strategy-momentum-trend");
    let factor_entry = harness.components.import("alice", &factor_bytes).unwrap();
    let strategy_entry = harness.components.import("alice", &strategy_bytes).unwrap();
    let factor_package = ComponentPackage::read(&factor_bytes).unwrap();
    let strategy_package = ComponentPackage::read(&strategy_bytes).unwrap();
    let mut mismatched_entry = factor_entry.clone();
    mismatched_entry.archive_sha256 = "c".repeat(64);
    let mut mismatched_package = factor_package.clone();
    mismatched_package.manifest.version = serde_json::from_str("\"0.2.0\"").unwrap();
    let mut incompatible_entry = factor_entry.clone();
    incompatible_entry.archive_sha256 = "d".repeat(64);
    incompatible_entry.compatible = false;
    let matches = compatible_factor_hashes(
        &strategy_package.manifest,
        &[
            (&factor_entry, factor_package),
            (&mismatched_entry, mismatched_package),
            (
                &incompatible_entry,
                ComponentPackage::read(&factor_bytes).unwrap(),
            ),
        ],
    );
    assert_eq!(matches["momentum"], [factor_entry.archive_sha256.clone()]);
    // The module-level lookup agrees for the real library contents.
    assert_eq!(
        harness
            .components
            .compatible_factors("alice", &strategy_entry.archive_sha256)
            .unwrap()["momentum"],
        [factor_entry.archive_sha256]
    );
    finish(harness);
}

#[test]
fn deletion_locks_flow_through_the_backtest_module() {
    let harness = harness("deletion-lock");
    let (manifest, wasm) = fixture("strategy");
    let strategy = harness
        .components
        .import("alice", &pack_component(manifest, &wasm).unwrap())
        .unwrap();
    seed_snapshot(&harness, "alice", "snapshot-1");
    let run = harness
        .backtests
        .run(run_request("snapshot-1", &strategy.archive_sha256))
        .unwrap();

    let listed = harness.components.list("alice").unwrap();
    let locked = listed
        .iter()
        .find(|component| component.archive_sha256 == strategy.archive_sha256)
        .unwrap();
    assert_eq!(locked.locked_by_run_ids, [run.run_id.clone()]);
    assert_eq!(
        harness
            .components
            .delete("alice", &strategy.archive_sha256)
            .unwrap_err(),
        format!(
            "Component Package is locked by Backtest Run: {}",
            run.run_id
        )
    );

    harness.backtests.delete("alice", &run.run_id).unwrap();
    harness
        .components
        .delete("alice", &strategy.archive_sha256)
        .unwrap();
    assert!(
        !harness
            .components
            .is_imported("alice", &strategy.archive_sha256)
            .unwrap()
    );
    finish(harness);
}

#[test]
fn listing_uses_imported_metadata_without_rereading_the_archive() {
    let harness = harness("metadata");
    let (factor, wasm) = fixture("factor");
    let imported = harness
        .components
        .import("alice", &pack_component(factor, &wasm).unwrap())
        .unwrap();
    let (strategy, wasm) = fixture("strategy");
    fs::write(
        harness
            .source
            .archive_directory
            .join(format!("{}.adaq", imported.archive_sha256)),
        pack_component(strategy, &wasm).unwrap(),
    )
    .unwrap();

    let listed = harness.components.list("alice").unwrap();
    assert!(listed[0].compatible);
    assert_eq!(listed[0].archive_sha256, imported.archive_sha256);
    assert!(
        harness
            .components
            .package_for_user("alice", &imported.archive_sha256)
            .unwrap_err()
            .contains("does not match stored identity or hashes")
    );
    finish(harness);
}
