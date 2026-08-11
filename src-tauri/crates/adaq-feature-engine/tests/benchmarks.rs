//! M10.9 performance acceptance workloads.
//!
//! Two committed workloads prove scale semantics before final acceptance:
//! a 1,000,000-Bar Time-Series workload and a 10,000-Instrument ×
//! 252-Observation Cross-Sectional workload. Both stream evidence instead
//! of accumulating it (bounded memory), stay deterministic across chunk
//! partitions, and stop promptly on cancellation. The full workloads are
//! `#[ignore]` and run serially in release:
//!
//! ```sh
//! cargo test -p adaq-feature-engine --release --test benchmarks \
//!     -- --ignored --test-threads=1
//! ```
//!
//! Recording the canonical macOS ARM64 baseline (no invented budgets):
//!
//! ```sh
//! ADAQ_FEATURE_RECORD_BASELINE=1 cargo test -p adaq-feature-engine \
//!     --release --test benchmarks -- --ignored --test-threads=1
//! ```
//!
//! Runtimes are measured per workload; the recorded RSS is the process
//! high-water mark (`ru_maxrss`) of the serial benchmark process, because a
//! per-workload peak cannot be isolated inside one process.
//!
//! A reduced-scale smoke workload exercises the same determinism and
//! cancellation properties inside the default test suite.

use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use adaq_feature_engine::{
    DefinitionDraft, FeatureDefinition, FeatureEngine, FeatureEngineIdentity,
    FeatureEvaluationInput, FeatureInput, FeatureInputEvent, FeatureMarketBar,
    FeatureMarketContext, FeatureNode, FeatureOperator, FeatureOutput, FeaturePlan,
    FeaturePlanDraft, FeatureScope, MarketField, PointInTimeInstrumentUniverse,
    UniverseEvidenceState,
};
use rust_decimal::Decimal;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const TIME_SERIES_BARS: usize = 1_000_000;
const CROSS_SECTIONAL_INSTRUMENTS: usize = 10_000;
const CROSS_SECTIONAL_OBSERVATIONS: usize = 252;
const BASELINE_FILE: &str = "fixtures/feature-benchmark-baseline.json";

/// Serial-execution handoff: the Cross-Sectional test records its runtime
/// here so the baseline is written once, after both workloads ran in one
/// process, regardless of harness test order.
static CROSS_SECTIONAL_RUNTIME_MS: Mutex<Option<u128>> = Mutex::new(None);

fn identity() -> FeatureEngineIdentity {
    FeatureEngineIdentity::for_tests()
}

/// Deterministic xorshift64 generator: benchmark inputs never depend on
/// wall-clock time or platform randomness.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn next_bounded(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

fn ts_plan() -> FeaturePlan {
    let ts_node =
        |id: &str, operator: FeatureOperator, parameters: BTreeMap<String, serde_json::Value>| {
            FeatureNode {
                id: id.into(),
                operator,
                scope: FeatureScope::TimeSeries,
                inputs: vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                parameters,
                warmup_bars: 0,
            }
        };
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(0x_0000_0000_0000_0000_0000_0000_000b_0001),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![
            ts_node(
                "log-return",
                FeatureOperator::BackwardLogReturn,
                BTreeMap::new(),
            ),
            ts_node(
                "mean",
                FeatureOperator::RollingMean,
                BTreeMap::from([("window".into(), json!(20))]),
            ),
            ts_node(
                "realized",
                FeatureOperator::RealizedVolatility,
                BTreeMap::from([("window".into(), json!(20))]),
            ),
        ],
        outputs: vec![
            FeatureOutput {
                name: "log-return".into(),
                node_id: "log-return".into(),
            },
            FeatureOutput {
                name: "mean".into(),
                node_id: "mean".into(),
            },
            FeatureOutput {
                name: "realized".into(),
                node_id: "realized".into(),
            },
        ],
    })
    .unwrap();
    FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap()
}

/// Builds one deterministic bar for a stream index; chunks regenerate the
/// same sequence so chunk boundaries can never change output.
fn ts_event(plan_seed: u64, index: usize) -> FeatureInputEvent {
    let mut rng = Rng(plan_seed
        .wrapping_add(index as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        | 1);
    let cents = 10_000 + rng.next_bounded(90_000);
    let price = Decimal::new(cents as i64, 2);
    let volume = Decimal::new(1 + rng.next_bounded(1_000) as i64, 0);
    let time = 1_700_000_000_000 + index as i64 * 60_000;
    FeatureInputEvent::observation(FeatureEvaluationInput::new(
        "okx:BENCH-USDT",
        time,
        time,
        FeatureMarketBar {
            open_time_ms: time,
            open: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(price)),
            high: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(price)),
            low: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(price)),
            close: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(price)),
            base_volume: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(volume)),
            quote_volume: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(volume)),
        },
    ))
}

/// Streams the Time-Series workload chunk by chunk, digesting every
/// Observation batch and dropping it: live evidence never grows beyond one
/// chunk. Returns the canonical digest and the number of processed Bars.
fn stream_time_series(
    bar_count: usize,
    chunk_size: usize,
    cancel: Option<&AtomicBool>,
) -> (String, usize) {
    let plan = ts_plan();
    let engine = FeatureEngine::new(plan.engine_identity());
    let mut evaluator = engine.evaluator(plan).unwrap();
    let mut digest = Sha256::new();
    let mut processed = 0usize;
    while processed < bar_count {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            break;
        }
        let remaining = bar_count - processed;
        let chunk_len = chunk_size.min(remaining);
        let events: Vec<FeatureInputEvent> = (processed..processed + chunk_len)
            .map(|index| ts_event(0x5eed_0001, index))
            .collect();
        let observations = evaluator.evaluate_batch(&events).unwrap();
        for observation in &observations {
            let canonical =
                adaq_feature_engine::canonicalize_json(&serde_json::to_vec(observation).unwrap())
                    .unwrap();
            digest.update(&canonical);
        }
        processed += chunk_len;
    }
    (
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        processed,
    )
}

fn cs_plan() -> FeaturePlan {
    let cs_node = |id: &str, operator: FeatureOperator| FeatureNode {
        id: id.into(),
        operator,
        scope: FeatureScope::CrossSectional,
        inputs: vec![FeatureInput::Market {
            field: MarketField::Close,
        }],
        parameters: BTreeMap::new(),
        warmup_bars: 0,
    };
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(0x_0000_0000_0000_0000_0000_0000_000b_0002),
        revision: 1,
        scope: FeatureScope::CrossSectional,
        nodes: vec![
            cs_node("rank", FeatureOperator::CrossSectionalRank),
            cs_node("percentile", FeatureOperator::CrossSectionalPercentile),
            cs_node("z-score", FeatureOperator::CrossSectionalZScore),
        ],
        outputs: vec![
            FeatureOutput {
                name: "rank".into(),
                node_id: "rank".into(),
            },
            FeatureOutput {
                name: "percentile".into(),
                node_id: "percentile".into(),
            },
            FeatureOutput {
                name: "z-score".into(),
                node_id: "z-score".into(),
            },
        ],
    })
    .unwrap();
    FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap()
}

/// Streams the Cross-Sectional workload one complete Observation-Time batch
/// at a time: memory stays bounded by one Universe batch, ordering stays
/// deterministic, and cancellation stops between batches.
fn stream_cross_sectional(
    instruments: usize,
    observation_count: usize,
    cancel: Option<&AtomicBool>,
) -> (String, usize) {
    let plan = cs_plan();
    let engine = FeatureEngine::new(plan.engine_identity());
    let mut evaluator = engine.evaluator(plan).unwrap();
    let members: Vec<String> = (0..instruments)
        .map(|index| format!("iex:BENCH-{index:06}"))
        .collect();
    let context = FeatureMarketContext::new(
        adaq_data_core::market::Venue::us_equity("iex").unwrap(),
        adaq_data_core::market::VenueKind::UsEquity,
        adaq_data_core::BarInterval::OneDay,
        adaq_data_core::market::PriceBasis::Unadjusted,
        "USD",
    )
    .unwrap();
    let mut digest = Sha256::new();
    let mut processed_batches = 0usize;
    for observation_index in 0..observation_count {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            break;
        }
        let time = 1_700_000_000_000 + observation_index as i64 * 86_400_000;
        // One complete Point-in-Time Universe per Observation Time: the
        // batch identity binds the exact batch instant.
        let universe = PointInTimeInstrumentUniverse::new(
            "benchmark-universe",
            time,
            members.clone(),
            context.clone(),
            UniverseEvidenceState::Observed,
        )
        .unwrap();
        let mut rng = Rng(0xC5_0000_0000 | (observation_index as u64 + 1));
        let inputs: Vec<FeatureEvaluationInput> = members
            .iter()
            .map(|member| {
                let cents = 500 + rng.next_bounded(99_500);
                let price = Decimal::new(cents as i64, 2);
                FeatureEvaluationInput::new(
                    member,
                    time,
                    time,
                    FeatureMarketBar {
                        open_time_ms: time,
                        open: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(price)),
                        high: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(price)),
                        low: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(price)),
                        close: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(price)),
                        base_volume: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(
                            Decimal::ONE,
                        )),
                        quote_volume: Some(adaq_feature_engine::CanonicalDecimal::from_decimal(
                            Decimal::ONE,
                        )),
                    },
                )
                .with_market_context(context.clone())
            })
            .collect();
        let observations = evaluator
            .observe(FeatureInputEvent::cross_sectional_batch(
                time,
                universe.clone(),
                inputs,
            ))
            .unwrap();
        for observation in &observations {
            let canonical =
                adaq_feature_engine::canonicalize_json(&serde_json::to_vec(observation).unwrap())
                    .unwrap();
            digest.update(&canonical);
        }
        processed_batches += 1;
    }
    (
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        processed_batches,
    )
}

#[cfg(unix)]
fn peak_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        let usage = unsafe { usage.assume_init() };
        // macOS reports bytes; Linux reports kibibytes. The canonical
        // baseline records macOS ARM64, so document the unit explicitly.
        #[cfg(target_os = "macos")]
        {
            return usage.ru_maxrss as u64;
        }
        #[cfg(not(target_os = "macos"))]
        {
            return usage.ru_maxrss as u64 * 1024;
        }
    }
    0
}

#[cfg(not(unix))]
fn peak_rss_bytes() -> u64 {
    0
}

fn baseline_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(BASELINE_FILE)
}

fn record_baseline(ts_runtime_ms: u128, cs_runtime_ms: u128) {
    let baseline = json!({
        "schemaVersion": "adaq-feature-benchmark-baseline@1.0.0",
        "recordedPlatform": std::env::consts::OS,
        "recordedArchitecture": std::env::consts::ARCH,
        "targetTriple": env!("ADAQ_FEATURE_ENGINE_TARGET"),
        "timeSeries": {
            "bars": TIME_SERIES_BARS,
            "outputs": 3,
            "runtimeMs": ts_runtime_ms
        },
        "crossSectional": {
            "instruments": CROSS_SECTIONAL_INSTRUMENTS,
            "observations": CROSS_SECTIONAL_OBSERVATIONS,
            "outputs": 3,
            "runtimeMs": cs_runtime_ms
        },
        "peakProcessRssBytes": peak_rss_bytes(),
        "rssSemantics": "high-water RSS (ru_maxrss) of the serial release benchmark process"
    });
    let mut json = serde_json::to_string_pretty(&baseline).unwrap();
    json.push('\n');
    std::fs::write(baseline_path(), json).unwrap();
}

// ---------------------------------------------------------------------------
// Default-suite smoke workload: same properties at reduced scale
// ---------------------------------------------------------------------------

#[test]
fn time_series_smoke_is_deterministic_across_chunks_and_cancellable() {
    let (full, processed) = stream_time_series(25_000, 8_192, None);
    assert_eq!(processed, 25_000);
    let (rechunked, _) = stream_time_series(25_000, 4_096, None);
    assert_eq!(full, rechunked, "chunk size must never change output");

    let cancel = AtomicBool::new(false);
    let (_, processed) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| stream_time_series(1_000_000, 8_192, Some(&cancel)));
        std::thread::sleep(std::time::Duration::from_millis(25));
        cancel.store(true, Ordering::Relaxed);
        handle.join().unwrap()
    });
    assert!(
        processed < 1_000_000,
        "cancellation must stop streaming before the workload completes"
    );
}

#[test]
fn cross_sectional_smoke_is_deterministic_and_cancellable() {
    let (full, batches) = stream_cross_sectional(200, 12, None);
    assert_eq!(batches, 12);
    let (replayed, _) = stream_cross_sectional(200, 12, None);
    assert_eq!(full, replayed, "batch ordering must stay deterministic");

    let cancel = AtomicBool::new(false);
    cancel.store(true, Ordering::Relaxed);
    let (_, batches) = stream_cross_sectional(200, 12, Some(&cancel));
    assert_eq!(batches, 0, "a cancelled workload processes no batches");
}

#[test]
fn committed_benchmark_baseline_describes_the_m109_workloads() {
    let raw = std::fs::read_to_string(baseline_path())
        .expect("the canonical benchmark baseline must be committed");
    let baseline: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        baseline["schemaVersion"],
        "adaq-feature-benchmark-baseline@1.0.0"
    );
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
}

// ---------------------------------------------------------------------------
// Full M10.9 acceptance workloads (release profile)
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn time_series_one_million_bars_streams_with_determinism_and_cancellation() {
    let started = Instant::now();
    let (full, processed) = stream_time_series(TIME_SERIES_BARS, 8_192, None);
    let runtime_ms = started.elapsed().as_millis();
    assert_eq!(processed, TIME_SERIES_BARS);

    // Deterministic chunks: a different partition reproduces the digest.
    let (rechunked, _) = stream_time_series(TIME_SERIES_BARS, 4_096, None);
    assert_eq!(full, rechunked);

    // Cancellation reaches the streaming loop between chunks.
    let cancel = AtomicBool::new(false);
    let cancel_started = Instant::now();
    let (_, processed) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| stream_time_series(TIME_SERIES_BARS, 8_192, Some(&cancel)));
        std::thread::sleep(std::time::Duration::from_millis(100));
        cancel.store(true, Ordering::Relaxed);
        handle.join().unwrap()
    });
    let stop_latency_ms = cancel_started.elapsed().as_millis();
    assert!(processed < TIME_SERIES_BARS);
    assert!(
        stop_latency_ms < 5_000,
        "cancellation stopped the workload {stop_latency_ms}ms after start"
    );

    println!(
        "feature-benchmark time-series bars={TIME_SERIES_BARS} runtime_ms={runtime_ms} \
         cancellation_stop_ms={stop_latency_ms}"
    );
    if std::env::var("ADAQ_FEATURE_RECORD_BASELINE").as_deref() == Ok("1") {
        // The baseline is written once, after both workloads ran serially,
        // so the recorded RSS reflects the complete benchmark process.
        let cs_runtime_ms = match *CROSS_SECTIONAL_RUNTIME_MS.lock().unwrap() {
            Some(runtime) => runtime,
            None => run_cross_sectional_full(),
        };
        record_baseline(runtime_ms, cs_runtime_ms);
    }
}

fn run_cross_sectional_full() -> u128 {
    let started = Instant::now();
    let (full, batches) = stream_cross_sectional(
        CROSS_SECTIONAL_INSTRUMENTS,
        CROSS_SECTIONAL_OBSERVATIONS,
        None,
    );
    let runtime_ms = started.elapsed().as_millis();
    assert_eq!(batches, CROSS_SECTIONAL_OBSERVATIONS);
    let (replayed, _) = stream_cross_sectional(
        CROSS_SECTIONAL_INSTRUMENTS,
        CROSS_SECTIONAL_OBSERVATIONS,
        None,
    );
    assert_eq!(full, replayed);
    let cancel = AtomicBool::new(true);
    let (_, cancelled_batches) = stream_cross_sectional(
        CROSS_SECTIONAL_INSTRUMENTS,
        CROSS_SECTIONAL_OBSERVATIONS,
        Some(&cancel),
    );
    assert_eq!(cancelled_batches, 0);
    println!(
        "feature-benchmark cross-sectional instruments={CROSS_SECTIONAL_INSTRUMENTS} \
         observations={CROSS_SECTIONAL_OBSERVATIONS} runtime_ms={runtime_ms}"
    );
    *CROSS_SECTIONAL_RUNTIME_MS.lock().unwrap() = Some(runtime_ms);
    runtime_ms
}

#[test]
#[ignore]
fn cross_sectional_ten_thousand_by_252_streams_per_batch() {
    run_cross_sectional_full();
}
