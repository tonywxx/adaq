//! M11.7 candidate-execution workloads.
//!
//! The ignored release test runs the canonical 1,000,000-row Time-Series and
//! 10,000-instrument x 252-observation Cross-Sectional workloads. Each call
//! retains only one candidate batch and a digest, so chunking, cancellation,
//! and restart replay stay observable without accumulating the Dataset.
//!
//! ```sh
//! cargo test -p adaq-factor-research --release --test benchmarks \
//!     -- --ignored --test-threads=1
//! ADAQ_FACTOR_RECORD_BASELINE=1 cargo test -p adaq-factor-research \
//!     --release --test benchmarks -- --ignored --test-threads=1
//! ```

use std::{
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use adaq_component_sdk::host::{factor_cross_sectional_abi, factor_time_series_abi};
use adaq_component_tooling::{ComponentPackage, RunLimits, WasmLoader};
use adaq_factor_research::{
    CandidateBuildRequest, FactorResourcePolicy, project_source_sha256,
    spawn_controlled_candidate_build,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const TIME_SERIES_BARS: usize = 1_000_000;
const CROSS_SECTIONAL_INSTRUMENTS: usize = 10_000;
const CROSS_SECTIONAL_OBSERVATIONS: usize = 252;
const TIME_SERIES_CHUNK: usize = 512;
const BASELINE_FILE: &str = "fixtures/factor-benchmark-baseline.json";

static TIME_SERIES_PACKAGE: OnceLock<ComponentPackage> = OnceLock::new();
static CROSS_SECTIONAL_PACKAGE: OnceLock<ComponentPackage> = OnceLock::new();

use factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api as cs;
use factor_time_series_abi::exports::adaq::factor::time_series_api as ts;

fn fixture_package(name: &str, attempt_id: u128) -> ComponentPackage {
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    let worker = spawn_controlled_candidate_build(CandidateBuildRequest {
        attempt_id: Uuid::from_u128(attempt_id),
        user_id: Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0094),
        source_sha256: project_source_sha256(&project_root).unwrap(),
        project_root,
        sdk_version: "0.1.0".into(),
        toolchain: "stable".into(),
        target: "wasm32-unknown-unknown".into(),
        resource_policy: FactorResourcePolicy {
            fuel_per_call: 10_000_000,
            memory_bytes: 64 * 1024 * 1024,
        },
    })
    .unwrap();
    worker.join().result.unwrap().package
}

fn time_series_package() -> &'static ComponentPackage {
    TIME_SERIES_PACKAGE
        .get_or_init(|| fixture_package("factor", 0x9400_0000_0000_0000_0000_0000_0000_0095))
}

fn cross_sectional_package() -> &'static ComponentPackage {
    CROSS_SECTIONAL_PACKAGE.get_or_init(|| {
        fixture_package(
            "cross-sectional-factor",
            0x9400_0000_0000_0000_0000_0000_0000_0096,
        )
    })
}

fn time_series_row(index: usize) -> ts::TimeSeriesRow {
    let time = index as i64;
    ts::TimeSeriesRow {
        instrument_id: "BENCH-USDT".into(),
        observation_time_ms: time,
        slots: vec![
            ts::FeatureValue {
                value: 100.0 + (index % 10_000) as f64,
                available_at_ms: time,
            },
            ts::FeatureValue {
                value: 1.0 + (index % 100) as f64,
                available_at_ms: time,
            },
        ],
    }
}

fn time_series_digest(results: &[ts::FactorResult], digest: &mut Sha256) {
    for result in results {
        digest.update(result.instrument_id.as_bytes());
        digest.update(result.observation_time_ms.to_le_bytes());
        match &result.values {
            None => digest.update([0]),
            Some(values) => {
                digest.update([1]);
                for value in values {
                    digest.update(value.name.as_bytes());
                    digest.update(value.value.to_bits().to_le_bytes());
                }
            }
        }
    }
}

fn run_time_series(bars: usize, chunk_size: usize, cancel: Option<&AtomicBool>) -> (String, usize) {
    let loader = WasmLoader::with_limits(RunLimits {
        fuel_per_call: 1_000_000_000,
        max_bars: chunk_size,
        ..RunLimits::default()
    });
    loader
        .load_factor_time_series_bytes(&time_series_package().wasm, Vec::new(), &[])
        .unwrap();
    let mut digest = Sha256::new();
    let mut processed = 0;
    while processed < bars {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            break;
        }
        let end = (processed + chunk_size).min(bars);
        let rows = (processed..end).map(time_series_row).collect();
        let results = loader.process_factor(rows).unwrap();
        time_series_digest(&results, &mut digest);
        processed = end;
    }
    (hex_digest(digest), processed)
}

fn cross_sectional_row(instrument: usize, time: usize) -> cs::CrossSectionalRow {
    cs::CrossSectionalRow {
        instrument_id: format!("BENCH-{instrument:05}"),
        observation_time_ms: time as i64,
        slots: vec![cs::FeatureCell::Available(cs::FeatureValue {
            value: 100.0 + instrument as f64 * 0.001 + time as f64 * 0.01,
            available_at_ms: time as i64,
        })],
    }
}

fn cross_sectional_digest(results: &[cs::FactorResult], digest: &mut Sha256) {
    for result in results {
        digest.update(result.instrument_id.as_bytes());
        digest.update(result.observation_time_ms.to_le_bytes());
        match &result.values {
            None => digest.update([0]),
            Some(values) => {
                digest.update([1]);
                for value in values {
                    digest.update(value.name.as_bytes());
                    digest.update(value.value.to_bits().to_le_bytes());
                }
            }
        }
    }
}

fn run_cross_sectional(
    instruments: usize,
    observations: usize,
    cancel: Option<&AtomicBool>,
) -> (String, usize) {
    let loader = WasmLoader::with_limits(RunLimits {
        fuel_per_call: 1_000_000_000,
        ..RunLimits::default()
    });
    loader
        .load_factor_cross_sectional_bytes(&cross_sectional_package().wasm, Vec::new(), &[])
        .unwrap();
    let expected = (0..instruments)
        .map(|instrument| format!("BENCH-{instrument:05}"))
        .collect::<Vec<_>>();
    let mut digest = Sha256::new();
    let mut processed = 0;
    while processed < observations {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            break;
        }
        let rows = (0..instruments)
            .map(|instrument| cross_sectional_row(instrument, processed))
            .collect();
        let results = loader
            .process_cross_sectional_factor(rows, &expected)
            .unwrap();
        cross_sectional_digest(&results, &mut digest);
        processed += 1;
    }
    (hex_digest(digest), processed)
}

fn hex_digest(digest: Sha256) -> String {
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
fn peak_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        let usage = unsafe { usage.assume_init() };
        #[cfg(target_os = "macos")]
        {
            usage.ru_maxrss as u64
        }
        #[cfg(not(target_os = "macos"))]
        {
            usage.ru_maxrss as u64 * 1024
        }
    } else {
        0
    }
}

#[cfg(not(unix))]
fn peak_rss_bytes() -> u64 {
    0
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(BASELINE_FILE)
}

fn target_triple() -> &'static str {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(not(any(
        all(target_arch = "aarch64", target_os = "macos"),
        all(target_arch = "x86_64", target_os = "windows"),
        all(target_arch = "x86_64", target_os = "linux")
    )))]
    {
        "unknown"
    }
}

fn record_baseline(
    ts_runtime_ms: u128,
    cs_runtime_ms: u128,
    ts_package: &ComponentPackage,
    cs_package: &ComponentPackage,
) {
    let baseline = serde_json::json!({
        "schemaVersion": "adaq-factor-benchmark-baseline@1.0.0",
        "recordedPlatform": std::env::consts::OS,
        "recordedArchitecture": std::env::consts::ARCH,
        "targetTriple": target_triple(),
        "timeSeries": {
            "bars": TIME_SERIES_BARS,
            "chunkRows": TIME_SERIES_CHUNK,
            "outputs": 1,
            "runtimeMs": ts_runtime_ms,
            "candidatePackageSha256": ts_package.archive_sha256,
        },
        "crossSectional": {
            "instruments": CROSS_SECTIONAL_INSTRUMENTS,
            "observations": CROSS_SECTIONAL_OBSERVATIONS,
            "chunkRows": CROSS_SECTIONAL_INSTRUMENTS,
            "outputs": 1,
            "runtimeMs": cs_runtime_ms,
            "candidatePackageSha256": cs_package.archive_sha256,
        },
        "limits": {
            "maxDatasetRows": adaq_factor_research::MAX_FACTOR_DATASET_ROWS,
            "maxEvaluationFolds": adaq_factor_research::MAX_FACTOR_EVALUATION_FOLDS,
            "maxEvaluationHorizons": adaq_factor_research::MAX_FACTOR_EVALUATION_HORIZONS,
            "maxEvaluationLenses": adaq_factor_research::MAX_FACTOR_EVALUATION_LENSES,
            "maxNuisanceFeatures": adaq_factor_research::MAX_FACTOR_NUISANCE_FEATURES,
            "workerCount": adaq_factor_research::MAX_FACTOR_WORKERS,
        },
        "peakProcessRssBytes": peak_rss_bytes(),
        "rssSemantics": "serial process high-water RSS (ru_maxrss)",
    });
    let mut bytes = serde_json::to_vec_pretty(&baseline).unwrap();
    bytes.push(b'\n');
    std::fs::write(baseline_path(), bytes).unwrap();
}

#[test]
fn candidate_batches_are_deterministic_and_cancellable() {
    let (full, processed) = run_time_series(25_000, TIME_SERIES_CHUNK, None);
    assert_eq!(processed, 25_000);
    let (replayed, _) = run_time_series(25_000, 4_096, None);
    assert_eq!(
        full, replayed,
        "Time-Series chunking changed candidate output"
    );

    let cancelled = AtomicBool::new(true);
    let (_, processed) = run_time_series(25_000, TIME_SERIES_CHUNK, Some(&cancelled));
    assert_eq!(processed, 0);

    let (full, processed) = run_cross_sectional(200, 12, None);
    assert_eq!(processed, 12);
    let (replayed, _) = run_cross_sectional(200, 12, None);
    assert_eq!(
        full, replayed,
        "Cross-Sectional batch replay changed candidate output"
    );

    let cancelled = AtomicBool::new(true);
    let (_, processed) = run_cross_sectional(200, 12, Some(&cancelled));
    assert_eq!(processed, 0);
}

#[test]
fn committed_benchmark_baseline_describes_factor_workloads_and_limits() {
    let baseline: serde_json::Value = serde_json::from_slice(
        &std::fs::read(baseline_path()).expect("committed Factor benchmark baseline is required"),
    )
    .unwrap();
    assert_eq!(
        baseline["schemaVersion"],
        "adaq-factor-benchmark-baseline@1.0.0"
    );
    assert!(baseline["targetTriple"].as_str().is_some());
    assert_eq!(baseline["timeSeries"]["bars"], TIME_SERIES_BARS as u64);
    assert_eq!(
        baseline["crossSectional"]["instruments"],
        CROSS_SECTIONAL_INSTRUMENTS as u64
    );
    assert_eq!(
        baseline["crossSectional"]["observations"],
        CROSS_SECTIONAL_OBSERVATIONS as u64
    );
    assert!(baseline["timeSeries"]["runtimeMs"].as_u64().is_some());
    assert!(baseline["crossSectional"]["runtimeMs"].as_u64().is_some());
    assert!(baseline["peakProcessRssBytes"].as_u64().is_some());
    assert_eq!(
        baseline["limits"]["maxDatasetRows"],
        adaq_factor_research::MAX_FACTOR_DATASET_ROWS as u64
    );
    for package_hash in [
        &baseline["timeSeries"]["candidatePackageSha256"],
        &baseline["crossSectional"]["candidatePackageSha256"],
    ] {
        assert_eq!(package_hash.as_str().map(str::len), Some(64));
    }
}

#[test]
#[ignore]
fn canonical_factor_workloads_run_with_replay_and_cancellation() {
    let ts_package = time_series_package();
    let ts_started = Instant::now();
    let (ts_digest, ts_processed) = run_time_series(TIME_SERIES_BARS, TIME_SERIES_CHUNK, None);
    let ts_runtime_ms = ts_started.elapsed().as_millis();
    assert_eq!(ts_processed, TIME_SERIES_BARS);
    let (replayed, _) = run_time_series(TIME_SERIES_BARS, 4_096, None);
    assert_eq!(ts_digest, replayed);

    let ts_cancel = AtomicBool::new(false);
    let (_, cancelled) = std::thread::scope(|scope| {
        let handle =
            scope.spawn(|| run_time_series(TIME_SERIES_BARS, TIME_SERIES_CHUNK, Some(&ts_cancel)));
        std::thread::sleep(std::time::Duration::from_millis(100));
        ts_cancel.store(true, Ordering::Relaxed);
        handle.join().unwrap()
    });
    assert!(cancelled < TIME_SERIES_BARS);

    let cs_package = cross_sectional_package();
    let cs_started = Instant::now();
    let (cs_digest, cs_processed) = run_cross_sectional(
        CROSS_SECTIONAL_INSTRUMENTS,
        CROSS_SECTIONAL_OBSERVATIONS,
        None,
    );
    let cs_runtime_ms = cs_started.elapsed().as_millis();
    assert_eq!(cs_processed, CROSS_SECTIONAL_OBSERVATIONS);
    let (replayed, _) = run_cross_sectional(
        CROSS_SECTIONAL_INSTRUMENTS,
        CROSS_SECTIONAL_OBSERVATIONS,
        None,
    );
    assert_eq!(cs_digest, replayed);

    let cs_cancel = AtomicBool::new(true);
    let (_, cancelled) = run_cross_sectional(
        CROSS_SECTIONAL_INSTRUMENTS,
        CROSS_SECTIONAL_OBSERVATIONS,
        Some(&cs_cancel),
    );
    assert_eq!(cancelled, 0);

    println!(
        "factor-benchmark ts_bars={TIME_SERIES_BARS} ts_runtime_ms={ts_runtime_ms} \
         cs_instruments={CROSS_SECTIONAL_INSTRUMENTS} cs_observations={CROSS_SECTIONAL_OBSERVATIONS} \
         cs_runtime_ms={cs_runtime_ms}"
    );
    if std::env::var("ADAQ_FACTOR_RECORD_BASELINE").as_deref() == Ok("1") {
        record_baseline(ts_runtime_ms, cs_runtime_ms, ts_package, cs_package);
    }
}
