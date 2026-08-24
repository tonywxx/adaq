//! Committed three-market reference fixtures for the Feature Engine.
//!
//! These journeys freeze the M10.9 acceptance semantics for OKX Spot,
//! China A-share, and U.S. equity evidence shapes: exact Definition and
//! Plan identities, canonical observation digests, and hand-pinned
//! samples cross-checked by independent reference implementations.
//! Every platform build evaluates the identical fixture inputs, so the
//! committed vectors double as cross-platform numerical equivalence
//! evidence (1e-12 tolerance where real math differs).
//!
//! Regeneration is deterministic and no-diff checked in CI:
//! `ADAQ_FEATURE_REGENERATE=1 cargo test -p adaq-feature-engine
//!  --test reference_fixtures regenerate` rewrites
//! `fixtures/feature-reference-vectors.json`.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use adaq_data_core::{
    BarInterval,
    market::{
        DayEvidence, PriceBasis, SessionPhase, TradingCalendarSnapshot, TradingDate,
        TradingSession, Venue, VenueKind,
    },
};
use adaq_feature_engine::{
    CanonicalDecimal, CorporateAction, DefinitionDraft, FeatureDefinition, FeatureEngine,
    FeatureEngineIdentity, FeatureEvaluationInput, FeatureInput, FeatureInputEvent,
    FeatureMarketBar, FeatureMarketContext, FeatureNode, FeatureObservation,
    FeatureObservationValue, FeatureOperator, FeatureOutput, FeaturePlan, FeaturePlanDraft,
    FeatureReference, FeatureScope, FeatureUnavailabilityReason, FittingAlgorithm, FittingScope,
    MarketField, ObservationRange, PointInTimeInstrumentUniverse, TransformationFittingProtocol,
    TransformationFittingProtocolDraft, UniverseEvidenceState,
};
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

const VECTORS_FILE: &str = "fixtures/feature-reference-vectors.json";
const VECTORS_SCHEMA: &str = "adaq-feature-reference-vectors@1.0.0";
const TOLERANCE: f64 = 1e-12;

// ---------------------------------------------------------------------------
// Committed vector contracts
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferenceVectors {
    schema_version: String,
    journeys: BTreeMap<String, JourneyVectors>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JourneyVectors {
    definition_hashes: Vec<String>,
    plan_hash: String,
    observations_sha256: String,
    samples: Vec<SampleVector>,
    #[serde(default)]
    protocol_hashes: Vec<String>,
    #[serde(default)]
    artifact_hashes: Vec<String>,
    #[serde(default)]
    error_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SampleVector {
    output: String,
    instrument: String,
    time_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    available_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coverage_available_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coverage_evidence_state: Option<String>,
}

/// One journey's runtime evidence before vector extraction.
struct Journey {
    definitions: Vec<FeatureDefinition>,
    plan: FeaturePlan,
    observations: Vec<FeatureObservation>,
    samples: Vec<SampleVector>,
    protocol_hashes: Vec<String>,
    artifact_hashes: Vec<String>,
    error_codes: Vec<String>,
}

fn vectors_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTORS_FILE)
}

// ---------------------------------------------------------------------------
// Shared builders
// ---------------------------------------------------------------------------

fn identity() -> FeatureEngineIdentity {
    FeatureEngineIdentity::for_tests()
}

fn definition_id(seed: u128) -> Uuid {
    Uuid::from_u128(seed)
}

fn ts_node(
    id: &str,
    operator: FeatureOperator,
    inputs: Vec<FeatureInput>,
    parameters: BTreeMap<String, serde_json::Value>,
) -> FeatureNode {
    FeatureNode {
        id: id.into(),
        operator,
        scope: FeatureScope::TimeSeries,
        inputs,
        parameters,
        warmup_bars: 0,
    }
}

fn market_input(field: MarketField) -> FeatureInput {
    FeatureInput::Market { field }
}

fn outputs(pairs: &[(&str, &str)]) -> Vec<FeatureOutput> {
    pairs
        .iter()
        .map(|(name, node_id)| FeatureOutput {
            name: (*name).into(),
            node_id: (*node_id).into(),
        })
        .collect()
}

fn freeze_ts_definition(
    seed: u128,
    nodes: Vec<FeatureNode>,
    out: &[(&str, &str)],
) -> FeatureDefinition {
    FeatureDefinition::freeze(DefinitionDraft {
        definition_id: definition_id(seed),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes,
        outputs: outputs(out),
    })
    .unwrap()
}

fn freeze_cs_definition(
    seed: u128,
    nodes: Vec<FeatureNode>,
    out: &[(&str, &str)],
) -> FeatureDefinition {
    FeatureDefinition::freeze(DefinitionDraft {
        definition_id: definition_id(seed),
        revision: 1,
        scope: FeatureScope::CrossSectional,
        nodes,
        outputs: outputs(out),
    })
    .unwrap()
}

fn freeze_plan(definitions: Vec<FeatureDefinition>) -> FeaturePlan {
    FeaturePlan::freeze(FeaturePlanDraft {
        definitions,
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap()
}

fn full_bar(time: i64, close: &str, base_volume: &str, quote_volume: &str) -> FeatureMarketBar {
    FeatureMarketBar::complete(time, close, close, close, close, base_volume, quote_volume).unwrap()
}

fn observation(instrument: &str, time: i64, bar: FeatureMarketBar) -> FeatureInputEvent {
    FeatureInputEvent::observation(FeatureEvaluationInput::new(instrument, time, time, bar))
}

fn value_of(observation: &FeatureObservation) -> Option<f64> {
    match observation.value {
        FeatureObservationValue::Available { value, .. } => Some(value),
        FeatureObservationValue::Unavailable { .. } => None,
    }
}

fn reason_of(observation: &FeatureObservation) -> Option<FeatureUnavailabilityReason> {
    observation.reason()
}

fn available_at_of(observation: &FeatureObservation) -> Option<i64> {
    match observation.value {
        FeatureObservationValue::Available {
            available_at_ms, ..
        } => Some(available_at_ms),
        FeatureObservationValue::Unavailable { .. } => None,
    }
}

fn find<'a>(
    observations: &'a [FeatureObservation],
    output: &str,
    instrument: &str,
    time_ms: i64,
) -> &'a FeatureObservation {
    observations
        .iter()
        .find(|observation| {
            observation.output_name == output
                && observation.instrument_id == instrument
                && observation.observation_time_ms == time_ms
        })
        .unwrap_or_else(|| panic!("missing observation {output}/{instrument}@{time_ms}"))
}

fn sample(
    observations: &[FeatureObservation],
    output: &str,
    instrument: &str,
    time_ms: i64,
) -> SampleVector {
    let observation = find(observations, output, instrument, time_ms);
    SampleVector {
        output: output.into(),
        instrument: instrument.into(),
        time_ms,
        value: value_of(observation),
        reason: reason_of(observation).map(|reason| reason.code().to_owned()),
        available_at_ms: available_at_of(observation),
        coverage_available_count: observation
            .cross_sectional_coverage
            .as_ref()
            .map(|coverage| coverage.available_count),
        coverage_evidence_state: observation
            .cross_sectional_coverage
            .as_ref()
            .map(|coverage| match coverage.evidence_state {
                UniverseEvidenceState::Observed => "observed".to_owned(),
                UniverseEvidenceState::Reconstructed => "reconstructed".to_owned(),
                UniverseEvidenceState::Unknown => "unknown".to_owned(),
            }),
    }
}

fn quantize_json_for_digest(value: &mut serde_json::Value) {
    // Committed digests must reproduce on every supported platform, but
    // transcendental engine math (`ln`) delegates to the platform libm and
    // can differ by up to one ulp between macOS, Windows, and Linux. Float
    // values are therefore reduced to seven significant digits before
    // hashing; Rust's own float formatting and parsing are deterministic,
    // integer fields stay exact, and sample comparisons still use the
    // strict 1e-12 tolerance.
    match value {
        serde_json::Value::Number(number) => {
            if number.as_i64().is_none()
                && number.as_u64().is_none()
                && let Some(float) = number.as_f64()
                && float.is_finite()
                && let Some(quantized) = format!("{float:.6e}")
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
            {
                *number = quantized;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                quantize_json_for_digest(item);
            }
        }
        serde_json::Value::Object(entries) => {
            for (_, entry) in entries.iter_mut() {
                quantize_json_for_digest(entry);
            }
        }
        _ => {}
    }
}

fn canonical_digest(value: serde_json::Value) -> String {
    let mut value = value;
    quantize_json_for_digest(&mut value);
    let canonical =
        adaq_feature_engine::canonicalize_json(&serde_json::to_vec(&value).unwrap()).unwrap();
    adaq_feature_engine::sha256(&canonical)
}

fn observations_digest(observations: &[FeatureObservation]) -> String {
    canonical_digest(serde_json::to_value(observations).unwrap())
}

fn artifact_digest(artifact: &adaq_feature_engine::FittedTransformationArtifact) -> String {
    canonical_digest(serde_json::from_slice(&artifact.to_json()).unwrap())
}

fn journey_vectors(journey: &Journey) -> JourneyVectors {
    // The committed Plan hash always derives from the test identity so it
    // is stable across build platforms; journeys that evaluate Indicator
    // nodes still run against the native engine identity, and their
    // observation digest pins the platform-independent values.
    let stable_plan = freeze_plan(journey.definitions.clone());
    JourneyVectors {
        definition_hashes: journey
            .definitions
            .iter()
            .map(|definition| definition.definition_hash().to_owned())
            .collect(),
        plan_hash: stable_plan.plan_hash().to_owned(),
        observations_sha256: observations_digest(&journey.observations),
        samples: journey.samples.clone(),
        protocol_hashes: journey.protocol_hashes.clone(),
        artifact_hashes: journey.artifact_hashes.clone(),
        error_codes: journey.error_codes.clone(),
    }
}

// ---------------------------------------------------------------------------
// Journey 1: OKX Spot
// ---------------------------------------------------------------------------

/// OKX Spot minute bars: backward returns, RSI through the pinned Indicator
/// Engine, Realized Volatility, one Bar Gap reset, and one missing-field
/// branch. Instrument `okx:BTC-USDT`.
fn okx_spot_journey() -> Journey {
    let instrument = "okx:BTC-USDT";
    let base = 1_710_000_000_000i64;
    let minute = 60_000i64;
    let definition = freeze_ts_definition(
        0x_0000_0000_0000_0000_0000_0000_0000_0001,
        vec![
            ts_node(
                "simple-return",
                FeatureOperator::BackwardSimpleReturn,
                vec![market_input(MarketField::Close)],
                BTreeMap::new(),
            ),
            ts_node(
                "log-return",
                FeatureOperator::BackwardLogReturn,
                vec![market_input(MarketField::Close)],
                BTreeMap::new(),
            ),
            ts_node(
                "rsi",
                FeatureOperator::Indicator { id: "rsi".into() },
                vec![market_input(MarketField::Close)],
                BTreeMap::from([
                    ("output".into(), json!("value")),
                    ("time-period".into(), json!(3)),
                ]),
            ),
            ts_node(
                "realized",
                FeatureOperator::RealizedVolatility,
                vec![market_input(MarketField::Close)],
                BTreeMap::from([("window".into(), json!(3))]),
            ),
            ts_node(
                "quote",
                FeatureOperator::QuoteVolume,
                vec![market_input(MarketField::QuoteVolume)],
                BTreeMap::new(),
            ),
        ],
        &[
            ("simple-return", "simple-return"),
            ("log-return", "log-return"),
            ("rsi", "rsi"),
            ("realized", "realized"),
            ("quote", "quote"),
        ],
    );
    // Indicator nodes execute through the pinned native Indicator Engine,
    // so the evaluated Plan carries the native identity; the committed
    // vector hash still derives from the stable test identity.
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition.clone()],
        engine_identity: FeatureEngineIdentity::native().unwrap(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();

    // Closes before the gap; the gap resets every stateful branch.
    let closes_before = ["100", "101.5", "99.25", "102.75"];
    // After the gap; index 6 (105.5) lacks Quote Volume to exercise the
    // missing-field branch without disturbing close-derived branches.
    let closes_after = [
        "103", "101", "105.5", "104.25", "106.75", "108", "107.5", "109.25",
    ];
    let mut events = Vec::new();
    for (index, close) in closes_before.iter().enumerate() {
        let time = base + index as i64 * minute;
        events.push(observation(
            instrument,
            time,
            full_bar(time, close, "1", "1000"),
        ));
    }
    events.push(FeatureInputEvent::bar_gap(
        instrument,
        base + closes_before.len() as i64 * minute,
        base + closes_before.len() as i64 * minute,
    ));
    for (index, close) in closes_after.iter().enumerate() {
        let time = base + (closes_before.len() + 1 + index) as i64 * minute;
        let bar = if index == 2 {
            FeatureMarketBar {
                open_time_ms: time,
                open: Some(CanonicalDecimal::new(*close).unwrap()),
                high: Some(CanonicalDecimal::new(*close).unwrap()),
                low: Some(CanonicalDecimal::new(*close).unwrap()),
                close: Some(CanonicalDecimal::new(*close).unwrap()),
                base_volume: Some(CanonicalDecimal::new("1").unwrap()),
                quote_volume: None,
            }
        } else {
            full_bar(time, close, "1", "1000")
        };
        events.push(observation(instrument, time, bar));
    }

    let engine = FeatureEngine::new(plan.engine_identity());
    let observations = engine.evaluate_batch(plan.clone(), &events).unwrap();

    let gap_time = base + closes_before.len() as i64 * minute;
    let t0 = base;
    let t1 = base + minute;
    let first_after_gap = base + (closes_before.len() + 1) as i64 * minute;
    let missing_quote_time = base + (closes_before.len() + 3) as i64 * minute;
    let next_quote_time = base + (closes_before.len() + 4) as i64 * minute;
    let final_time = base + (closes_before.len() + closes_after.len()) as i64 * minute;

    let samples = vec![
        sample(&observations, "simple-return", instrument, t0),
        sample(&observations, "simple-return", instrument, t1),
        sample(&observations, "log-return", instrument, t1),
        sample(&observations, "simple-return", instrument, gap_time),
        sample(&observations, "realized", instrument, gap_time),
        sample(&observations, "simple-return", instrument, first_after_gap),
        sample(&observations, "quote", instrument, missing_quote_time),
        sample(&observations, "quote", instrument, next_quote_time),
        sample(&observations, "rsi", instrument, final_time),
        sample(&observations, "realized", instrument, final_time),
    ];

    Journey {
        definitions: vec![definition],
        plan,
        observations,
        samples,
        protocol_hashes: Vec::new(),
        artifact_hashes: Vec::new(),
        error_codes: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Journey 2: China A-share
// ---------------------------------------------------------------------------

fn ashare_calendar(venue: &Venue) -> TradingCalendarSnapshot {
    TradingCalendarSnapshot::new(
        "m109-a-share-reference",
        venue.clone(),
        0,
        2_000_000_000_000,
        vec![
            TradingSession {
                phase: SessionPhase::Continuous,
                start_local: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
                end_local: NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
            },
            TradingSession {
                phase: SessionPhase::Break,
                start_local: NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
                end_local: NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
            },
            TradingSession {
                phase: SessionPhase::Continuous,
                start_local: NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
                end_local: NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            },
        ],
        Vec::<DayEvidence>::new(),
    )
    .unwrap()
}

/// China A-share: IANA venue calendar with a midday break Session Progress,
/// causal Split adjustment, Dividend Total Return, and publication-delayed
/// availability. Canonical decimal inputs must survive evaluation unchanged.
fn china_a_share_journey() -> Journey {
    let venue = Venue::china_a_share("sse").unwrap();
    let instrument = "sse:600000";
    let calendar = ashare_calendar(&venue);
    // Monday 2024-03-11, venue-local 10:00 = 09:00-09:30 block after open.
    let local = |hour: u32, minute: u32| {
        let date = TradingDate::from_utc_ms(&venue, 1_710_126_000_000).unwrap();
        venue
            .resolve_local_time(
                date.to_naive_date()
                    .unwrap()
                    .and_hms_opt(hour, minute, 0)
                    .unwrap(),
                adaq_data_core::market::LocalTimeDisambiguation::Reject,
            )
            .unwrap()
    };
    let definition = freeze_ts_definition(
        0x_0000_0000_0000_0000_0000_0000_0000_0002,
        vec![
            ts_node(
                "progress",
                FeatureOperator::SessionProgress,
                Vec::new(),
                BTreeMap::new(),
            ),
            ts_node(
                "from-open",
                FeatureOperator::MinutesFromSessionOpen,
                Vec::new(),
                BTreeMap::new(),
            ),
            ts_node(
                "to-close",
                FeatureOperator::MinutesToSessionClose,
                Vec::new(),
                BTreeMap::new(),
            ),
            ts_node(
                "day",
                FeatureOperator::TradingDayOfWeek,
                Vec::new(),
                BTreeMap::new(),
            ),
            ts_node(
                "month",
                FeatureOperator::TradingMonth,
                Vec::new(),
                BTreeMap::new(),
            ),
            ts_node(
                "split",
                FeatureOperator::CausalSplitAdjustment,
                vec![market_input(MarketField::Close)],
                BTreeMap::new(),
            ),
            ts_node(
                "dividend",
                FeatureOperator::DividendTotalReturn,
                vec![market_input(MarketField::Close)],
                BTreeMap::new(),
            ),
        ],
        &[
            ("progress", "progress"),
            ("from-open", "from-open"),
            ("to-close", "to-close"),
            ("day", "day"),
            ("month", "month"),
            ("split", "split"),
            ("dividend", "dividend"),
        ],
    );
    let plan = freeze_plan(vec![definition.clone()]);

    let t_open = local(9, 30);
    let t_mid_morning = local(10, 0);
    let t_lunch_start = local(11, 30);
    let t_afternoon = local(13, 30);
    let t_pre_close = local(14, 30);

    // The Split is effective at the afternoon observation but published
    // later: availability must track the publication instant, not the
    // effective instant or local computation time.
    let split_effective = t_afternoon;
    let split_available = t_afternoon + 2 * 60_000;
    let split = CorporateAction::split_with_evidence(
        instrument,
        "fixture:m109:split:600000",
        split_effective,
        split_available,
        "1",
    )
    .unwrap();
    let dividend_effective = t_pre_close;
    let dividend = CorporateAction::dividend_with_evidence(
        instrument,
        "fixture:m109:dividend:600000",
        dividend_effective,
        dividend_effective,
        "5",
        Some(CanonicalDecimal::new("100").unwrap()),
    )
    .unwrap();

    let mut events = vec![
        FeatureInputEvent::observation(
            FeatureEvaluationInput::new(
                instrument,
                t_open,
                t_open,
                full_bar(t_open, "10", "1", "10"),
            )
            .with_calendar(calendar.clone()),
        ),
        FeatureInputEvent::observation(
            FeatureEvaluationInput::new(
                instrument,
                t_mid_morning,
                t_mid_morning,
                full_bar(t_mid_morning, "10.2", "1", "10"),
            )
            .with_calendar(calendar.clone()),
        ),
        FeatureInputEvent::observation(
            FeatureEvaluationInput::new(
                instrument,
                t_lunch_start,
                t_lunch_start,
                full_bar(t_lunch_start, "10.1", "1", "10"),
            )
            .with_calendar(calendar.clone()),
        ),
        FeatureInputEvent::observation(
            FeatureEvaluationInput::new(
                instrument,
                t_afternoon,
                t_afternoon,
                full_bar(t_afternoon, "5.1", "1", "10"),
            )
            .with_calendar(calendar.clone())
            .with_corporate_actions(vec![split]),
        ),
        FeatureInputEvent::observation(
            FeatureEvaluationInput::new(
                instrument,
                t_pre_close,
                t_pre_close,
                full_bar(t_pre_close, "5.2", "1", "10"),
            )
            .with_calendar(calendar.clone())
            .with_corporate_actions(vec![dividend]),
        ),
    ];
    // Canonical inputs must never be mutated by evaluation: keep an exact
    // copy of every decimal string before evaluation and compare after.
    let canonical_before = events
        .iter()
        .map(|event| match event {
            FeatureInputEvent::Observation(input) => input.bar.as_ref().and_then(|bar| {
                bar.close
                    .as_ref()
                    .map(|decimal| decimal.as_str().to_owned())
            }),
            _ => None,
        })
        .collect::<Vec<_>>();

    let engine = FeatureEngine::new(identity());
    let observations = engine.evaluate_batch(plan.clone(), &events).unwrap();

    let canonical_after = events
        .iter()
        .map(|event| match event {
            FeatureInputEvent::Observation(input) => input.bar.as_ref().and_then(|bar| {
                bar.close
                    .as_ref()
                    .map(|decimal| decimal.as_str().to_owned())
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        canonical_before, canonical_after,
        "evaluation must never mutate Canonical Market Data inputs"
    );
    events.clear();

    let samples = vec![
        sample(&observations, "from-open", instrument, t_mid_morning),
        sample(&observations, "to-close", instrument, t_mid_morning),
        sample(&observations, "progress", instrument, t_mid_morning),
        sample(&observations, "progress", instrument, t_lunch_start),
        sample(&observations, "from-open", instrument, t_afternoon),
        sample(&observations, "day", instrument, t_open),
        sample(&observations, "month", instrument, t_open),
        sample(&observations, "split", instrument, t_lunch_start),
        sample(&observations, "split", instrument, t_afternoon),
        sample(&observations, "split", instrument, t_pre_close),
        sample(&observations, "dividend", instrument, t_afternoon),
        sample(&observations, "dividend", instrument, t_pre_close),
    ];

    Journey {
        definitions: vec![definition],
        plan,
        observations,
        samples,
        protocol_hashes: Vec::new(),
        artifact_hashes: Vec::new(),
        error_codes: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Journey 3: U.S. equities
// ---------------------------------------------------------------------------

fn us_market_context() -> FeatureMarketContext {
    FeatureMarketContext::new(
        Venue::us_equity("iex").unwrap(),
        VenueKind::UsEquity,
        BarInterval::OneDay,
        PriceBasis::Unadjusted,
        "USD",
    )
    .unwrap()
}

fn us_universe(
    time: i64,
    members: &[&str],
    evidence_state: UniverseEvidenceState,
) -> PointInTimeInstrumentUniverse {
    PointInTimeInstrumentUniverse::new(
        "fixture-m109-us-universe",
        time,
        members.iter().map(|member| (*member).to_owned()).collect(),
        us_market_context(),
        evidence_state,
    )
    .unwrap()
}

fn us_batch(
    time: i64,
    universe: PointInTimeInstrumentUniverse,
    values: &[(&str, &str)],
) -> FeatureInputEvent {
    let context = us_market_context();
    FeatureInputEvent::cross_sectional_batch(
        time,
        universe,
        values
            .iter()
            .map(|(instrument, close)| {
                FeatureEvaluationInput::new(
                    *instrument,
                    time,
                    time,
                    full_bar(time, close, "1", "1"),
                )
                .with_market_context(context.clone())
            })
            .collect(),
    )
}

fn cs_node(
    id: &str,
    operator: FeatureOperator,
    parameters: BTreeMap<String, serde_json::Value>,
) -> FeatureNode {
    FeatureNode {
        id: id.into(),
        operator,
        scope: FeatureScope::CrossSectional,
        inputs: vec![market_input(MarketField::Close)],
        parameters,
        warmup_bars: 0,
    }
}

/// U.S. equities: complete Point-in-Time Universe with deterministic
/// Rank/Percentile/Z-score, an explicit coverage threshold over a
/// Reconstructed Universe, Unknown Universe unavailability, and
/// mixed-market batch rejection.
fn us_equities_journey() -> Journey {
    let definition = freeze_cs_definition(
        0x_0000_0000_0000_0000_0000_0000_0000_0003,
        vec![
            cs_node("rank", FeatureOperator::CrossSectionalRank, BTreeMap::new()),
            cs_node(
                "reverse-rank",
                FeatureOperator::CrossSectionalRank,
                BTreeMap::from([("reverse".into(), json!(true))]),
            ),
            cs_node(
                "percentile",
                FeatureOperator::CrossSectionalPercentile,
                BTreeMap::new(),
            ),
            cs_node(
                "z-score",
                FeatureOperator::CrossSectionalZScore,
                BTreeMap::new(),
            ),
            cs_node(
                "relaxed-rank",
                FeatureOperator::CrossSectionalRank,
                BTreeMap::from([
                    ("minimum-count".into(), json!(2)),
                    ("minimum-coverage".into(), json!(0.6)),
                ]),
            ),
        ],
        &[
            ("rank", "rank"),
            ("reverse-rank", "reverse-rank"),
            ("percentile", "percentile"),
            ("z-score", "z-score"),
            ("relaxed-rank", "relaxed-rank"),
        ],
    );
    let plan = freeze_plan(vec![definition.clone()]);
    let members = ["AAPL", "MSFT", "NVDA", "TSLA", "XOM"];

    let observed = us_batch(
        100,
        us_universe(100, &members, UniverseEvidenceState::Observed),
        &[
            ("AAPL", "10"),
            ("MSFT", "20"),
            ("NVDA", "20"),
            ("TSLA", "40"),
            ("XOM", "50"),
        ],
    );
    // Reconstructed Universe with only three of five members available.
    let reconstructed = us_batch(
        200,
        us_universe(200, &members, UniverseEvidenceState::Reconstructed),
        &[("AAPL", "11"), ("NVDA", "21"), ("XOM", "31")],
    );
    // Unknown evidence state makes the complete batch Unavailable.
    let unknown = us_batch(
        300,
        us_universe(300, &members, UniverseEvidenceState::Unknown),
        &[("AAPL", "12")],
    );

    let engine = FeatureEngine::new(identity());
    let observations = engine
        .evaluate_batch(plan.clone(), &[observed, reconstructed, unknown])
        .unwrap();

    // Mixed-market batches are a fatal typed evaluation error, never an
    // Unavailable observation.
    let crypto_context = FeatureMarketContext::new(
        Venue::crypto_spot("okx").unwrap(),
        VenueKind::CryptoSpot,
        BarInterval::OneDay,
        PriceBasis::Unadjusted,
        "USD",
    )
    .unwrap();
    let mixed = FeatureInputEvent::cross_sectional_batch(
        400,
        us_universe(400, &["AAPL", "BTC-USDT"], UniverseEvidenceState::Observed),
        vec![
            FeatureEvaluationInput::new("AAPL", 400, 400, full_bar(400, "10", "1", "1"))
                .with_market_context(us_market_context()),
            FeatureEvaluationInput::new("BTC-USDT", 400, 400, full_bar(400, "20", "1", "1"))
                .with_market_context(crypto_context),
        ],
    );
    let mixed_error = FeatureEngine::new(identity())
        .evaluator(plan.clone())
        .unwrap()
        .observe(mixed)
        .unwrap_err();

    let samples = vec![
        sample(&observations, "rank", "AAPL", 100),
        sample(&observations, "rank", "NVDA", 100),
        sample(&observations, "reverse-rank", "NVDA", 100),
        sample(&observations, "percentile", "AAPL", 100),
        sample(&observations, "percentile", "TSLA", 100),
        sample(&observations, "z-score", "AAPL", 100),
        sample(&observations, "rank", "AAPL", 200),
        sample(&observations, "rank", "MSFT", 200),
        sample(&observations, "relaxed-rank", "AAPL", 200),
        sample(&observations, "relaxed-rank", "MSFT", 200),
        sample(&observations, "rank", "AAPL", 300),
        sample(&observations, "rank", "XOM", 300),
    ];

    Journey {
        definitions: vec![definition],
        plan,
        observations,
        samples,
        protocol_hashes: Vec::new(),
        artifact_hashes: Vec::new(),
        error_codes: vec![
            mixed_error.code().to_owned(),
            mixed_error.diagnostic.clone(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Journey 4: Fitted transformations
// ---------------------------------------------------------------------------

/// Fitting: exact Protocol identity, Pooled and Per-Instrument parameters,
/// walk-forward fold isolation, insufficient samples, Artifact reuse, and
/// leakage rejection.
fn fitting_journey() -> Journey {
    let instrument_a = "iex:FIT-A";
    let instrument_b = "iex:FIT-B";
    let definition = freeze_ts_definition(
        0x_0000_0000_0000_0000_0000_0000_0000_0004,
        vec![ts_node(
            "log-return",
            FeatureOperator::BackwardLogReturn,
            vec![market_input(MarketField::Close)],
            BTreeMap::new(),
        )],
        &[("log-return", "log-return")],
    );
    let plan = freeze_plan(vec![definition.clone()]);

    let closes_a = ["100", "101", "103", "102", "104", "106"];
    let closes_b = ["50", "49.5", "51", "52", "51.5", "53"];
    let mut events = Vec::new();
    for index in 0..closes_a.len() {
        let time = 1 + index as i64;
        events.push(observation(
            instrument_a,
            time,
            full_bar(time, closes_a[index], "1", "1"),
        ));
    }
    for index in 0..closes_b.len() {
        let time = 1 + index as i64;
        events.push(observation(
            instrument_b,
            time,
            full_bar(time, closes_b[index], "1", "1"),
        ));
    }
    let observations = FeatureEngine::new(identity())
        .evaluate_batch(plan.clone(), &events)
        .unwrap();

    let input_feature = FeatureReference {
        definition_hash: definition.definition_hash().to_owned(),
        node_id: "log-return".into(),
        output_name: "log-return".into(),
    };
    let protocol_draft = |scope: FittingScope,
                          algorithm: FittingAlgorithm,
                          window: (i64, i64),
                          minimum_samples: u64,
                          fold: u128| {
        TransformationFittingProtocolDraft {
            input_feature: input_feature.clone(),
            fitted_node_id: "standardize".into(),
            fitted_output: FeatureReference {
                definition_hash: definition.definition_hash().to_owned(),
                node_id: "standardize".into(),
                output_name: "standardized".into(),
            },
            snapshot_id: format!("fixture-m109-snapshot-{fold}"),
            point_in_time_universe_id: "fixture-m109-universe".into(),
            valuation_currency: String::new(),
            fitting_scope: scope,
            fitting_window: ObservationRange {
                start_time_ms: window.0,
                end_time_ms: window.1,
            },
            algorithm,
            minimum_samples,
            engine_identity: identity(),
        }
    };

    // Exact Protocol identity: Pooled Standardization over the full window.
    let pooled = TransformationFittingProtocol::freeze(protocol_draft(
        FittingScope::PooledUniverse,
        FittingAlgorithm::Standardization,
        (1, 7),
        4,
        1,
    ))
    .unwrap();
    let pooled_artifact = pooled.fit(&observations, 900).unwrap();

    // Per-Instrument Winsorization freezes distinct parameter sets.
    let per_instrument = TransformationFittingProtocol::freeze(protocol_draft(
        FittingScope::PerInstrument,
        FittingAlgorithm::Winsorization {
            lower_quantile: 0.2,
            upper_quantile: 0.8,
            quantile_method_version: "nearest-rank@1.0.0".into(),
        },
        (1, 7),
        4,
        2,
    ))
    .unwrap();
    let per_instrument_artifact = per_instrument.fit(&observations, 900).unwrap();

    // Fold isolation: two Protocols differing only in their fitting window
    // must carry distinct identities, and a fold-1 Artifact must refuse to
    // apply to fold-2 observations (leakage rejection).
    let fold_one = TransformationFittingProtocol::freeze(protocol_draft(
        FittingScope::PooledUniverse,
        FittingAlgorithm::Standardization,
        (1, 4),
        2,
        3,
    ))
    .unwrap();
    let fold_two = TransformationFittingProtocol::freeze(protocol_draft(
        FittingScope::PooledUniverse,
        FittingAlgorithm::Standardization,
        (4, 7),
        2,
        4,
    ))
    .unwrap();
    assert_ne!(fold_one.protocol_hash(), fold_two.protocol_hash());
    let fold_one_artifact = fold_one.fit(&observations, 900).unwrap();
    let fold_two_artifact = fold_two.fit(&observations, 900).unwrap();
    // Leakage rejection: no fold Artifact may apply inside its own fitting
    // window (fold one) or before the predecessor fold has closed
    // (fold two at a fold-one observation time).
    let leakage = fold_one_artifact.apply_value(instrument_a, 2, 0.01, 2);
    assert!(matches!(
        leakage,
        Err(adaq_feature_engine::FittingApplyError::ArtifactNotAvailableForObservation { .. })
    ));
    let leakage = fold_two_artifact.apply_value(instrument_a, 3, 0.01, 3);
    assert!(matches!(
        leakage,
        Err(adaq_feature_engine::FittingApplyError::ArtifactNotAvailableForObservation { .. })
    ));

    // Artifact reuse: applying the same Artifact twice is identical.
    let eligible_time = 7;
    let first_apply = pooled_artifact
        .apply_value(instrument_a, eligible_time, 0.01, eligible_time)
        .unwrap();
    let second_apply = pooled_artifact
        .apply_value(instrument_a, eligible_time, 0.01, eligible_time)
        .unwrap();
    assert_eq!(format!("{first_apply:?}"), format!("{second_apply:?}"));

    // Insufficient samples fail without publishing an Artifact.
    let starved = TransformationFittingProtocol::freeze(protocol_draft(
        FittingScope::PooledUniverse,
        FittingAlgorithm::Standardization,
        (1, 7),
        100,
        5,
    ))
    .unwrap();
    let starved_error = starved.fit(&observations, 900).unwrap_err();

    Journey {
        definitions: vec![definition],
        plan,
        observations,
        samples: Vec::new(),
        protocol_hashes: vec![
            pooled.protocol_hash().to_owned(),
            per_instrument.protocol_hash().to_owned(),
            fold_one.protocol_hash().to_owned(),
            fold_two.protocol_hash().to_owned(),
            starved.protocol_hash().to_owned(),
        ],
        artifact_hashes: vec![
            artifact_digest(&pooled_artifact),
            artifact_digest(&per_instrument_artifact),
            artifact_digest(&fold_one_artifact),
        ],
        error_codes: vec![
            starved_error.code().to_owned(),
            "artifact-not-available-for-observation".to_owned(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Journey 5: Adversarial failures
// ---------------------------------------------------------------------------

/// Failures: undefined arithmetic stays typed Unavailable; non-finite
/// outputs, broken continuity, and engine identity mismatches are fatal
/// typed Feature Evaluation Errors with stage and diagnostics.
fn failures_journey() -> Journey {
    let instrument = "okx:FAIL-USDT";
    let definition = freeze_ts_definition(
        0x_0000_0000_0000_0000_0000_0000_0000_0005,
        vec![ts_node(
            "log-return",
            FeatureOperator::BackwardLogReturn,
            vec![market_input(MarketField::Close)],
            BTreeMap::new(),
        )],
        &[("log-return", "log-return")],
    );
    let plan = freeze_plan(vec![definition.clone()]);

    let events = vec![
        observation(instrument, 1, full_bar(1, "0", "1", "1")),
        observation(instrument, 2, full_bar(2, "1", "1", "1")),
    ];
    let engine = FeatureEngine::new(identity());
    let observations = engine.evaluate_batch(plan.clone(), &events).unwrap();
    let mut error_codes = Vec::new();

    // Undefined arithmetic: log of zero is typed Unavailable, not fatal.
    assert_eq!(
        reason_of(&observations[1]),
        Some(FeatureUnavailabilityReason::UndefinedArithmetic)
    );

    // Non-finite values can never become observations.
    let non_finite = FeatureObservation::available("bad", instrument, 3, f64::NAN, 3);
    error_codes.push(non_finite.unwrap_err().code().to_owned());

    // Broken causality: an observation that moves Observation Time
    // backwards on a stateful evaluator is an invalid observation shape.
    let mut causal = FeatureEngine::new(identity())
        .evaluator(plan.clone())
        .unwrap();
    causal
        .evaluate_batch(&[
            observation(instrument, 1, full_bar(1, "1", "1", "1")),
            observation(instrument, 2, full_bar(2, "2", "1", "1")),
        ])
        .unwrap();
    let broken = causal.observe(observation(instrument, 2, full_bar(2, "3", "1", "1")));
    error_codes.push(broken.unwrap_err().code().to_owned());

    // Engine identity mismatch is rejected before any evaluation.
    let mut foreign = identity();
    foreign.engine_build_id = "9".repeat(64);
    let mismatch = FeatureEngine::new(foreign).evaluator(plan.clone());
    error_codes.push(mismatch.unwrap_err().code().to_owned());

    let samples = vec![
        sample(&observations, "log-return", instrument, 1),
        sample(&observations, "log-return", instrument, 2),
    ];
    Journey {
        definitions: vec![definition],
        plan,
        observations,
        samples,
        protocol_hashes: Vec::new(),
        artifact_hashes: Vec::new(),
        error_codes,
    }
}

// ---------------------------------------------------------------------------
// Vector extraction, regeneration, and verification
// ---------------------------------------------------------------------------

fn journey_names() -> [&'static str; 5] {
    [
        "okx-spot",
        "china-a-share",
        "us-equities",
        "fitting",
        "failures",
    ]
}

fn compute_vectors() -> ReferenceVectors {
    let journeys = [
        okx_spot_journey(),
        china_a_share_journey(),
        us_equities_journey(),
        fitting_journey(),
        failures_journey(),
    ];
    let mut map = BTreeMap::new();
    for (name, journey) in journey_names().into_iter().zip(journeys) {
        map.insert(name.to_owned(), journey_vectors(&journey));
    }
    ReferenceVectors {
        schema_version: VECTORS_SCHEMA.into(),
        journeys: map,
    }
}

/// Deterministic regeneration entry point for the CI no-diff check:
/// `ADAQ_FEATURE_REGENERATE=1 cargo test -p adaq-feature-engine
///  --test reference_fixtures regenerate_reference_vectors -- --ignored`.
#[test]
#[ignore]
fn regenerate_reference_vectors() {
    if std::env::var("ADAQ_FEATURE_REGENERATE").as_deref() != Ok("1") {
        panic!("set ADAQ_FEATURE_REGENERATE=1 to rewrite the committed vectors");
    }
    let vectors = compute_vectors();
    let mut json = serde_json::to_string_pretty(&vectors).unwrap();
    json.push('\n');
    let path = vectors_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, json).unwrap();
}

fn load_committed_vectors() -> ReferenceVectors {
    let raw = fs::read_to_string(vectors_path()).unwrap_or_else(|error| {
        panic!(
            "committed reference vectors missing at {VECTORS_FILE}: {error}; \
             regenerate with ADAQ_FEATURE_REGENERATE=1"
        )
    });
    serde_json::from_str(&raw).unwrap()
}

fn assert_sample_matches(actual: &SampleVector, committed: &SampleVector) {
    assert_eq!(
        (
            actual.output.as_str(),
            actual.instrument.as_str(),
            actual.time_ms
        ),
        (
            committed.output.as_str(),
            committed.instrument.as_str(),
            committed.time_ms
        ),
        "sample key drift"
    );
    assert_eq!(
        actual.reason, committed.reason,
        "reason drift for {:?}",
        committed.output
    );
    assert_eq!(actual.available_at_ms, committed.available_at_ms);
    assert_eq!(
        actual.coverage_available_count,
        committed.coverage_available_count
    );
    assert_eq!(
        actual.coverage_evidence_state,
        committed.coverage_evidence_state
    );
    match (actual.value, committed.value) {
        (Some(actual_value), Some(committed_value)) => {
            assert!(
                (actual_value - committed_value).abs() <= TOLERANCE,
                "{} {}/{} drifted: {actual_value} vs {committed_value}",
                committed.output,
                committed.instrument,
                committed.time_ms,
            );
        }
        (None, None) => {}
        _ => panic!("availability drift for {}", committed.output),
    }
}

#[test]
fn committed_reference_vectors_match_the_three_market_journeys() {
    let committed = load_committed_vectors();
    assert_eq!(committed.schema_version, VECTORS_SCHEMA);
    let journeys = [
        okx_spot_journey(),
        china_a_share_journey(),
        us_equities_journey(),
        fitting_journey(),
        failures_journey(),
    ];
    for (name, journey) in journey_names().into_iter().zip(journeys) {
        let vectors = committed
            .journeys
            .get(name)
            .unwrap_or_else(|| panic!("missing committed journey {name}"));
        let actual = journey_vectors(&journey);
        // Every Available value must be finite on every supported build:
        // non-finite engine output is a fatal evaluation error, never a
        // committed observation.
        for observation in &journey.observations {
            if let Some(value) = value_of(observation) {
                assert!(value.is_finite(), "{name} produced a non-finite value");
            }
        }
        assert_eq!(
            actual.definition_hashes, vectors.definition_hashes,
            "{name} definition hashes"
        );
        assert_eq!(actual.plan_hash, vectors.plan_hash, "{name} plan hash");
        assert_eq!(
            actual.observations_sha256, vectors.observations_sha256,
            "{name} observation digest"
        );
        assert_eq!(
            actual.samples.len(),
            vectors.samples.len(),
            "{name} sample count"
        );
        for (actual_sample, committed_sample) in actual.samples.iter().zip(&vectors.samples) {
            assert_sample_matches(actual_sample, committed_sample);
        }
        assert_eq!(
            actual.protocol_hashes, vectors.protocol_hashes,
            "{name} protocol hashes"
        );
        assert_eq!(
            actual.artifact_hashes, vectors.artifact_hashes,
            "{name} artifact hashes"
        );
        assert_eq!(
            actual.error_codes, vectors.error_codes,
            "{name} error codes"
        );
    }
}

#[test]
fn reference_vectors_are_deterministic_across_repeated_freeze_and_replay() {
    let first = compute_vectors();
    let second = compute_vectors();
    for name in journey_names() {
        let first = &first.journeys[name];
        let second = &second.journeys[name];
        assert_eq!(first.definition_hashes, second.definition_hashes);
        assert_eq!(first.plan_hash, second.plan_hash);
        assert_eq!(first.observations_sha256, second.observations_sha256);
        assert_eq!(first.protocol_hashes, second.protocol_hashes);
        assert_eq!(first.artifact_hashes, second.artifact_hashes);
    }
}

// ---------------------------------------------------------------------------
// Batch/chunk/observation equivalence over the committed fixture inputs
// ---------------------------------------------------------------------------

fn rebuild_okx_events() -> Vec<FeatureInputEvent> {
    // Mirrors okx_spot_journey's event construction so equivalence can be
    // replayed against fresh evaluator instances.
    let instrument = "okx:BTC-USDT";
    let base = 1_710_000_000_000i64;
    let minute = 60_000i64;
    let closes_before = ["100", "101.5", "99.25", "102.75"];
    let closes_after = [
        "103", "101", "105.5", "104.25", "106.75", "108", "107.5", "109.25",
    ];
    let mut events = Vec::new();
    for (index, close) in closes_before.iter().enumerate() {
        let time = base + index as i64 * minute;
        events.push(observation(
            instrument,
            time,
            full_bar(time, close, "1", "1000"),
        ));
    }
    events.push(FeatureInputEvent::bar_gap(
        instrument,
        base + closes_before.len() as i64 * minute,
        base + closes_before.len() as i64 * minute,
    ));
    for (index, close) in closes_after.iter().enumerate() {
        let time = base + (closes_before.len() + 1 + index) as i64 * minute;
        let bar = if index == 2 {
            FeatureMarketBar {
                open_time_ms: time,
                open: Some(CanonicalDecimal::new(*close).unwrap()),
                high: Some(CanonicalDecimal::new(*close).unwrap()),
                low: Some(CanonicalDecimal::new(*close).unwrap()),
                close: Some(CanonicalDecimal::new(*close).unwrap()),
                base_volume: Some(CanonicalDecimal::new("1").unwrap()),
                quote_volume: None,
            }
        } else {
            full_bar(time, close, "1", "1000")
        };
        events.push(observation(instrument, time, bar));
    }
    events
}

#[test]
fn okx_fixture_is_equivalent_across_batch_chunk_and_observation_paths() {
    let journey = okx_spot_journey();
    let events = rebuild_okx_events();
    let reference = &journey.observations;
    let engine = FeatureEngine::new(journey.plan.engine_identity());

    let mut stateful = engine.evaluator(journey.plan.clone()).unwrap();
    let mut one_at_a_time = Vec::new();
    for event in &events {
        one_at_a_time.extend(stateful.observe(event.clone()).unwrap());
    }
    assert_eq!(&one_at_a_time, reference);

    for chunk_size in [2usize, 3, 5, 7, events.len()] {
        let mut chunked = engine.evaluator(journey.plan.clone()).unwrap();
        let mut observations = Vec::new();
        for chunk in events.chunks(chunk_size) {
            observations.extend(chunked.evaluate_batch(chunk).unwrap());
        }
        assert_eq!(&observations, reference, "chunk size {chunk_size} drifted");
    }

    let replayed = FeatureEngine::new(journey.plan.engine_identity())
        .evaluate_batch(journey.plan.clone(), &events)
        .unwrap();
    assert_eq!(&replayed, reference);
}

// ---------------------------------------------------------------------------
// Independent reference implementations
// ---------------------------------------------------------------------------

fn reference_simple_return(previous: f64, current: f64) -> f64 {
    current / previous - 1.0
}

fn reference_log_return(previous: f64, current: f64) -> f64 {
    (current / previous).ln()
}

/// Wilder-smoothed RSI exactly as the pinned TA-Lib seeds it: SMA over the
/// first `period` changes, then the classic recurrence.
fn reference_rsi(closes: &[f64], period: usize) -> f64 {
    let mut gains = 0.0f64;
    let mut losses = 0.0f64;
    for index in 1..=period {
        let change = closes[index] - closes[index - 1];
        if change > 0.0 {
            gains += change;
        } else {
            losses -= change;
        }
    }
    let mut average_gain = gains / period as f64;
    let mut average_loss = losses / period as f64;
    for index in period + 1..closes.len() {
        let change = closes[index] - closes[index - 1];
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);
        average_gain = (average_gain * (period as f64 - 1.0) + gain) / period as f64;
        average_loss = (average_loss * (period as f64 - 1.0) + loss) / period as f64;
    }
    if average_loss == 0.0 {
        return 100.0;
    }
    100.0 - 100.0 / (1.0 + average_gain / average_loss)
}

fn reference_realized_volatility(closes: &[f64], window: usize) -> f64 {
    // A full window counts `window` consecutive per-Bar log returns, i.e.
    // the last `window + 1` Closed Bars in one Continuous Bar Segment.
    let tail = &closes[closes.len() - (window + 1)..];
    let returns: Vec<f64> = tail
        .windows(2)
        .map(|pair| (pair[1] / pair[0]).ln())
        .collect();
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / returns.len() as f64;
    variance.sqrt()
}

fn reference_ascending_average_tie_ranks(values: &[(String, f64)]) -> HashMap<String, f64> {
    let mut order: Vec<&(String, f64)> = values.iter().collect();
    order.sort_by(|left, right| left.1.partial_cmp(&right.1).unwrap());
    let mut ranks = HashMap::new();
    let mut index = 0usize;
    while index < order.len() {
        let mut end = index + 1;
        while end < order.len() && order[end].1 == order[index].1 {
            end += 1;
        }
        let average = (index + 1 + end) as f64 / 2.0;
        for member in &order[index..end] {
            ranks.insert(member.0.clone(), average);
        }
        index = end;
    }
    ranks
}

#[test]
fn independent_reference_implementations_agree_with_the_engine() {
    let journey = okx_spot_journey();
    let instrument = "okx:BTC-USDT";
    let base = 1_710_000_000_000i64;
    let minute = 60_000i64;

    let closes: Vec<f64> = [
        "100", "101.5", "99.25", "102.75", "103", "101", "105.5", "104.25", "106.75", "108",
        "107.5", "109.25",
    ]
    .iter()
    .map(|value| value.parse::<f64>().unwrap())
    .collect();

    // Backward returns at the second bar are causal and exact.
    let simple = find(
        &journey.observations,
        "simple-return",
        instrument,
        base + minute,
    );
    let log = find(
        &journey.observations,
        "log-return",
        instrument,
        base + minute,
    );
    assert!(
        (value_of(simple).unwrap() - reference_simple_return(closes[0], closes[1])).abs()
            <= TOLERANCE
    );
    assert!(
        (value_of(log).unwrap() - reference_log_return(closes[0], closes[1])).abs() <= TOLERANCE
    );

    // RSI matches the independent Wilder implementation on the full
    // restart segment (the Bar Gap resets the indicator state).
    let final_time = base + 12 * minute;
    let rsi = value_of(find(&journey.observations, "rsi", instrument, final_time)).unwrap();
    let post_gap = &closes[4..];
    let expected_rsi = reference_rsi(post_gap, 3);
    assert!(
        (rsi - expected_rsi).abs() <= 1e-9,
        "RSI {rsi} != Wilder reference {expected_rsi}"
    );

    // Realized Volatility matches the independent per-bar log-return
    // population standard deviation on the restarted segment.
    let realized = value_of(find(
        &journey.observations,
        "realized",
        instrument,
        final_time,
    ))
    .unwrap();
    let expected_realized = reference_realized_volatility(post_gap, 3);
    assert!(
        (realized - expected_realized).abs() <= 1e-9,
        "realized volatility {realized} != reference {expected_realized}"
    );

    // Cross-sectional ranks use ascending average ties independently.
    let us = us_equities_journey();
    let independent = reference_ascending_average_tie_ranks(&[
        ("AAPL".into(), 10.0),
        ("MSFT".into(), 20.0),
        ("NVDA".into(), 20.0),
        ("TSLA".into(), 40.0),
        ("XOM".into(), 50.0),
    ]);
    for (member, expected) in &independent {
        let actual = value_of(find(&us.observations, "rank", member, 100)).unwrap();
        assert!(
            (actual - expected).abs() <= TOLERANCE,
            "rank for {member}: {actual} != {expected}"
        );
    }
    // Percentile is (rank - 1) / (n - 1).
    let n = 5.0f64;
    let percentile = value_of(find(&us.observations, "percentile", "TSLA", 100)).unwrap();
    let expected_percentile = (independent["TSLA"] - 1.0) / (n - 1.0);
    assert!((percentile - expected_percentile).abs() <= TOLERANCE);
    // Z-score uses population variance.
    let values = [10.0f64, 20.0, 20.0, 40.0, 50.0];
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    let zscore = value_of(find(&us.observations, "z-score", "AAPL", 100)).unwrap();
    let expected_zscore = (10.0 - mean) / variance.sqrt();
    assert!((zscore - expected_zscore).abs() <= TOLERANCE);
}

#[test]
fn china_a_share_availability_follows_publication_evidence() {
    let journey = china_a_share_journey();
    // The Split is effective at one Observation Time but published two
    // minutes later: the effective-time observation must stay Unavailable
    // with the corporate-action reason, and the first adjusted observation
    // must not claim availability before the publication instant.
    let withheld = journey
        .samples
        .iter()
        .find(|sample| {
            sample.output == "split"
                && sample.reason.as_deref() == Some("corporate-action-unavailable")
        })
        .expect("the effective-time split observation must be withheld");
    let publication_ms = withheld.time_ms + 2 * 60_000;
    let applied = journey
        .samples
        .iter()
        .find(|sample| {
            sample.output == "split"
                && sample.time_ms > withheld.time_ms
                && sample.available_at_ms.is_some()
        })
        .expect("a later adjusted split observation must be available");
    assert!(
        applied.available_at_ms.unwrap() >= publication_ms,
        "adjusted split availability must not precede publication evidence"
    );
    // The pre-effective observation stays unadjusted (no backward rewrite).
    let before = journey
        .samples
        .iter()
        .find(|sample| sample.output == "split" && sample.time_ms < withheld.time_ms)
        .expect("a pre-effective split observation must exist");
    assert!(before.reason.is_none());
    assert!(before.available_at_ms == Some(before.time_ms));
}

#[test]
fn native_feature_identity_is_exact_per_build_and_validates() {
    let native = adaq_feature_engine::FeatureEngineIdentity::native().unwrap();
    // The build embeds its own target triple; each supported CI build must
    // carry its exact identity rather than a shared placeholder.
    assert_eq!(native.target_triple, env!("ADAQ_FEATURE_ENGINE_TARGET"));
    assert!(native.feature_engine_source_sha256.len() == 64);
    assert!(native.feature_engine_build_id.len() == 64);
    assert_ne!(native.feature_engine_source_sha256, "f".repeat(64));
    // Structural equality with the indicator-derived construction path.
    let indicator = adaq_indicator_engine::IndicatorEngine::initialize().unwrap();
    let derived = adaq_feature_engine::FeatureEngineIdentity::from_indicator(indicator.identity());
    assert_eq!(native, derived);
}
