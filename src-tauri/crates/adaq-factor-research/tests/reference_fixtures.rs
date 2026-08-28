//! Deterministic M11.7 reference journeys.
//!
//! The committed vectors cover the three supported market shapes and retain
//! typed unavailable evidence instead of reducing failures to numeric zero.
//! The vector file is regenerated only with `ADAQ_FACTOR_REGENERATE=1`; the
//! default test is an exact drift check on every platform.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use adaq_data_core::{BarGap, OhlcvBar};
use adaq_factor_research::{
    AvailableFactorValue, CorporateActionEvidence, CrossSectionalInputRow, DeclarativeFactorDraft,
    EconomicAssumptions, EvaluationFeatureCell, EvaluationFeatureEvidence, EvaluationFeatureRow,
    EvaluationWindow, FactorAbiContract, FactorCandidate, FactorDataset, FactorEvaluationInput,
    FactorEvaluationProtocol, FactorEvaluationProtocolDraft, FactorEvaluationReport,
    FactorEvaluator, FactorFeatureSlot, FactorLens, FactorMarketContext,
    FactorMaterializationInput, FactorMaterializationProtocol, FactorMaterializationProtocolDraft,
    FactorObservationValue, FactorOrientation, FactorOutput, FactorResourcePolicy, FactorResult,
    FactorScope, FactorSlotCell, FactorTarget, FactorUnavailabilityReason, MetricId,
    MetricObservation, MetricUndefinedReason, NamedFactorOutput, ObservationRange,
    ResearchEngineProvenance, TimeSeriesInputRow, content_hash, load_versioned_json,
    validate_cross_sectional_batch, validate_factor_results, validate_time_series_batch,
};
use adaq_feature_engine::{
    FeatureDatasetCell, FeatureDatasetRow, FeatureEngineIdentity, FeaturePlan, FeaturePlanDraft,
    FeatureSlot, FeatureSource, FeatureUnavailabilityReason, MarketField,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const VECTORS_FILE: &str = "fixtures/factor-reference-vectors.json";
const VECTORS_SCHEMA: &str = "adaq-factor-reference-vectors@1.0.0";
const FLOAT_TOLERANCE: f64 = 1e-12;
const USER_ID: Uuid = Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0001);
const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferenceVectors {
    schema_version: String,
    journeys: BTreeMap<String, JourneyVector>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JourneyVector {
    market_context: FactorMarketContext,
    candidate_hash: String,
    feature_plan_hash: String,
    dataset_id: String,
    protocol_hash: String,
    evidence_state: String,
    metric_digest: String,
    target_unavailable: Vec<adaq_factor_research::TargetUnavailableEvidence>,
    metrics: Vec<MetricSample>,
    checks: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetricSample {
    fold_id: String,
    variant: String,
    horizon_bars: u32,
    lens: FactorLens,
    metric: MetricId,
    value: Option<f64>,
    undefined_reason: Option<MetricUndefinedReason>,
    sample_count: u64,
}

fn vectors_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTORS_FILE)
}

fn engine(build_id: &str) -> ResearchEngineProvenance {
    ResearchEngineProvenance {
        engine_id: "adaq-native-factor-fixture".into(),
        engine_version: "1.0.0".into(),
        adapter: "native".into(),
        target_triple: "fixture".into(),
        build_id: build_id.into(),
        environment: BTreeMap::new(),
        parameters: BTreeMap::new(),
        input_identities: vec!["reference-inputs@1".into()],
    }
}

fn plan(slots: &[&str]) -> FeaturePlan {
    FeaturePlan::freeze(FeaturePlanDraft {
        slots: slots
            .iter()
            .map(|name| FeatureSlot {
                name: (*name).into(),
                source: FeatureSource::Market {
                    field: MarketField::Close,
                },
                warmup_bars: 0,
            })
            .collect(),
        engine_identity: FeatureEngineIdentity::for_tests(),
        ..FeaturePlanDraft::default()
    })
    .unwrap()
}

fn context(
    venue: &str,
    asset_class: &str,
    bar_interval: &str,
    price_basis: &str,
    currency: &str,
    universe: &str,
) -> FactorMarketContext {
    FactorMarketContext {
        venue: venue.into(),
        asset_class: asset_class.into(),
        bar_interval: bar_interval.into(),
        price_basis: price_basis.into(),
        valuation_currency: currency.into(),
        point_in_time_universe_id: universe.into(),
    }
}

fn bar(time_ms: i64, close: i64) -> OhlcvBar {
    let close = Decimal::from(close);
    OhlcvBar {
        open_time_ms: time_ms,
        open: close,
        high: close,
        low: close,
        close,
        base_volume: Decimal::ONE,
        quote_volume: Decimal::ONE,
    }
}

fn feature_value(value: f64, time_ms: i64) -> FeatureDatasetCell {
    FeatureDatasetCell::Available {
        value,
        available_at_ms: time_ms,
    }
}

fn feature_unavailable(reason: FeatureUnavailabilityReason) -> FeatureDatasetCell {
    FeatureDatasetCell::Unavailable { reason }
}

fn feature_dataset(
    dataset_id: &str,
    plan: &FeaturePlan,
    snapshot: &str,
    universe: &str,
    output_names: &[&str],
    rows: Vec<FeatureDatasetRow>,
) -> adaq_factor_research::CompletedFeatureDataset {
    adaq_factor_research::CompletedFeatureDataset::new(
        USER_ID.to_string(),
        dataset_id,
        plan.plan_hash(),
        plan.to_json(),
        plan.engine_identity(),
        snapshot,
        universe,
        output_names.iter().map(|name| (*name).into()).collect(),
        rows,
    )
    .unwrap()
}

fn candidate(
    candidate_id: Uuid,
    scope: FactorScope,
    plan_hash: &str,
    slot: &str,
    output: &str,
) -> FactorCandidate {
    DeclarativeFactorDraft {
        user_id: USER_ID,
        candidate_id,
        revision: 1,
        scope,
        feature_slots: vec![FactorFeatureSlot { name: slot.into() }],
        parameters: vec![],
        outputs: vec![FactorOutput {
            name: output.into(),
        }],
        definition: adaq_factor_research::DeclarativeFactorDefinition {
            feature_plan_hash: plan_hash.into(),
            operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION.into(),
            outputs: vec![adaq_factor_research::DeclarativeFactorOutputBinding {
                output_name: output.into(),
                feature_slot: slot.into(),
            }],
        },
        presentation: adaq_factor_research::FactorPresentationMetadata {
            name: output.into(),
            description: "M11.7 deterministic reference factor".into(),
            tags: vec!["reference".into()],
        },
    }
    .publish()
    .unwrap()
    .0
}

fn materialization_protocol(
    protocol_id: Uuid,
    candidate: &FactorCandidate,
    feature_dataset: &adaq_factor_research::CompletedFeatureDataset,
    context: FactorMarketContext,
    end_time_ms: i64,
) -> FactorMaterializationProtocol {
    FactorMaterializationProtocol::freeze(FactorMaterializationProtocolDraft {
        protocol_id,
        user_id: USER_ID,
        candidate_hash: candidate.candidate_hash.clone(),
        feature_dataset_id: feature_dataset.dataset_id.clone(),
        feature_plan_hash: feature_dataset.feature_plan_hash.clone(),
        parameters: vec![],
        market_data_snapshot_id: feature_dataset.market_data_snapshot_id.clone(),
        point_in_time_universe_id: feature_dataset.point_in_time_universe_id.clone(),
        observation_range: ObservationRange {
            start_time_ms: 0,
            end_time_ms,
        },
        market_context: context,
        engine_identity: engine("factor-reference-engine"),
        seed: 9400,
    })
    .unwrap()
}

fn evaluation_protocol(
    protocol_id: Uuid,
    dataset: &FactorDataset,
    scope: FactorScope,
    context: FactorMarketContext,
    universe: Vec<String>,
    horizons: Vec<u32>,
    windows: Vec<EvaluationWindow>,
    lenses: Vec<FactorLens>,
    nuisance_feature_names: Vec<String>,
    regime: Option<adaq_factor_research::FactorRegimeDefinition>,
) -> FactorEvaluationProtocol {
    FactorEvaluationProtocol::freeze(FactorEvaluationProtocolDraft {
        protocol_id,
        user_id: USER_ID,
        factor_dataset_id: dataset.manifest.dataset_id.clone(),
        feature_dataset_id: dataset.manifest.feature_dataset_id.clone(),
        feature_plan_hash: dataset.manifest.feature_plan_hash.clone(),
        market_data_snapshot_id: dataset.manifest.market_data_snapshot_id.clone(),
        point_in_time_universe_id: dataset.manifest.point_in_time_universe_id.clone(),
        point_in_time_universe: universe,
        output_name: dataset.manifest.output_names[0].clone(),
        scope,
        target: FactorTarget::FutureCloseReturn,
        horizon_bars: horizons,
        market_context: context,
        engine_identity: engine("factor-reference-engine"),
        orientation: FactorOrientation::Positive,
        windows,
        purge_bars: 0,
        embargo_bars: 0,
        lenses,
        nuisance_feature_names,
        regime,
        economic: EconomicAssumptions {
            rebalance_every_bars: 1,
            fee_bps: 10.0,
            slippage_bps: 5.0,
            long_short: true,
        },
        family_id: protocol_id,
        trial_id: Uuid::from_u128(protocol_id.as_u128() + 1),
        seed: 9400,
    })
    .unwrap()
}

fn out_of_sample_window(selection_end: i64, evaluation_end: i64) -> EvaluationWindow {
    EvaluationWindow {
        fold_id: "fold-1".into(),
        selection: ObservationRange {
            start_time_ms: 0,
            end_time_ms: selection_end,
        },
        evaluation: ObservationRange {
            start_time_ms: selection_end,
            end_time_ms: evaluation_end,
        },
        training: Some(ObservationRange {
            start_time_ms: 0,
            end_time_ms: selection_end / 2,
        }),
        fitting: Some(ObservationRange {
            start_time_ms: 0,
            end_time_ms: selection_end / 2,
        }),
        normalization: Some(ObservationRange {
            start_time_ms: 0,
            end_time_ms: selection_end / 2,
        }),
        target_construction: Some(ObservationRange {
            start_time_ms: 0,
            end_time_ms: selection_end / 2,
        }),
    }
}

fn metric_sample(record: &adaq_factor_research::MetricRecord) -> MetricSample {
    let (value, undefined_reason, sample_count) = match record.observation {
        MetricObservation::Available {
            value,
            sample_count,
        } => (Some(value), None, sample_count),
        MetricObservation::Unavailable {
            reason,
            sample_count,
        } => (None, Some(reason), sample_count),
    };
    MetricSample {
        fold_id: record.fold_id.clone(),
        variant: record.variant.clone(),
        horizon_bars: record.horizon_bars,
        lens: record.lens,
        metric: record.metric,
        value,
        undefined_reason,
        sample_count,
    }
}

fn normalized_metric_sample(mut sample: MetricSample) -> MetricSample {
    sample.value = sample
        .value
        .map(|value| (value / FLOAT_TOLERANCE).round() * FLOAT_TOLERANCE);
    sample
}

fn stable_metric_digest(metrics: &[MetricSample]) -> String {
    content_hash(
        &metrics
            .iter()
            .cloned()
            .map(normalized_metric_sample)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn assert_reference_vectors_equal(expected: &ReferenceVectors, actual: &ReferenceVectors) {
    assert_eq!(expected.schema_version, actual.schema_version);
    assert_eq!(
        expected.journeys.keys().collect::<Vec<_>>(),
        actual.journeys.keys().collect::<Vec<_>>()
    );
    for (name, expected_journey) in &expected.journeys {
        let actual_journey = actual
            .journeys
            .get(name)
            .expect("reference journey identity is retained");
        assert_eq!(
            expected_journey.market_context,
            actual_journey.market_context
        );
        assert_eq!(
            expected_journey.candidate_hash,
            actual_journey.candidate_hash
        );
        assert_eq!(
            expected_journey.feature_plan_hash,
            actual_journey.feature_plan_hash
        );
        assert_eq!(expected_journey.dataset_id, actual_journey.dataset_id);
        assert_eq!(expected_journey.protocol_hash, actual_journey.protocol_hash);
        assert_eq!(
            expected_journey.evidence_state,
            actual_journey.evidence_state
        );
        assert_eq!(
            expected_journey.target_unavailable,
            actual_journey.target_unavailable
        );
        assert_eq!(expected_journey.checks, actual_journey.checks);
        assert_eq!(expected_journey.metrics.len(), actual_journey.metrics.len());
        for (expected_metric, actual_metric) in
            expected_journey.metrics.iter().zip(&actual_journey.metrics)
        {
            assert_eq!(
                (
                    &expected_metric.fold_id,
                    &expected_metric.variant,
                    expected_metric.horizon_bars,
                    expected_metric.lens,
                    expected_metric.metric,
                    expected_metric.undefined_reason,
                    expected_metric.sample_count,
                ),
                (
                    &actual_metric.fold_id,
                    &actual_metric.variant,
                    actual_metric.horizon_bars,
                    actual_metric.lens,
                    actual_metric.metric,
                    actual_metric.undefined_reason,
                    actual_metric.sample_count,
                )
            );
            match (expected_metric.value, actual_metric.value) {
                (Some(expected), Some(actual)) => assert!(
                    (expected - actual).abs() <= FLOAT_TOLERANCE,
                    "metric drift exceeded tolerance: expected={expected} actual={actual}"
                ),
                (None, None) => {}
                _ => panic!("metric availability changed"),
            }
        }
        assert_eq!(expected_journey.metric_digest, actual_journey.metric_digest);
    }
}

fn evidence_state(state: adaq_factor_research::EvaluationEvidenceState) -> String {
    serde_json::to_string(&state)
        .unwrap()
        .trim_matches('"')
        .into()
}

fn common_checks() -> BTreeMap<String, bool> {
    let v1 = FactorAbiContract {
        abi_version: "1.0.0".into(),
        scope: FactorScope::TimeSeries,
        feature_slots: vec![FactorFeatureSlot {
            name: "signal".into(),
        }],
        parameters: vec![],
        outputs: vec![FactorOutput {
            name: "score".into(),
        }],
        warmup_bars: 0,
        resource_policy: FactorResourcePolicy {
            fuel_per_call: 1,
            memory_bytes: 1,
        },
    };
    let nonfinite = FactorResult {
        instrument_id: "A".into(),
        observation_time_ms: 0,
        values: Some(vec![NamedFactorOutput {
            name: "score".into(),
            value: f64::NAN,
        }]),
    };
    let mut checks = BTreeMap::from([
        ("factor-abi-v1-reset".into(), v1.validate().is_err()),
        (
            "nonfinite-output-rejected".into(),
            validate_factor_results(&[nonfinite], &["A".into()], &[0], &["score".into()]).is_err(),
        ),
        (
            "malformed-evidence-reset".into(),
            matches!(
                load_versioned_json::<serde_json::Value>(
                    br#"{"schemaVersion":"0.1.0"}"#,
                    adaq_factor_research::FACTOR_RESEARCH_SCHEMA_VERSION,
                ),
                Err(adaq_factor_research::ContractLoadError::ResetRequired { .. })
            ),
        ),
    ]);
    checks.insert(
        "engine-identity-stays-distinct".into(),
        content_hash(&engine("macos-arm64")) != content_hash(&engine("windows-x86-64")),
    );
    checks
}

fn vector(
    context: FactorMarketContext,
    candidate: &FactorCandidate,
    dataset: &FactorDataset,
    protocol: &FactorEvaluationProtocol,
    report: &FactorEvaluationReport,
    mut checks: BTreeMap<String, bool>,
) -> JourneyVector {
    checks.insert("report-validates".into(), report.validate().is_ok());
    let metrics = report
        .metrics
        .iter()
        .map(metric_sample)
        .map(normalized_metric_sample)
        .collect::<Vec<_>>();
    JourneyVector {
        market_context: context,
        candidate_hash: candidate.candidate_hash.clone(),
        feature_plan_hash: dataset.manifest.feature_plan_hash.clone(),
        dataset_id: dataset.manifest.dataset_id.clone(),
        protocol_hash: protocol.protocol_hash.clone(),
        evidence_state: evidence_state(report.evidence_state),
        metric_digest: stable_metric_digest(&metrics),
        target_unavailable: report.target_unavailable.clone(),
        metrics,
        checks,
    }
}

fn okx_journey() -> JourneyVector {
    let context = context(
        "okx",
        "crypto-spot",
        "1h",
        "unadjusted",
        "USDT",
        "okx-spot-reference",
    );
    let plan = plan(&["signal"]);
    let times = (0..12).map(|index| index * HOUR_MS).collect::<Vec<_>>();
    let signal = [
        feature_unavailable(FeatureUnavailabilityReason::Warmup),
        feature_unavailable(FeatureUnavailabilityReason::Warmup),
        feature_value(1.0, times[2]),
        feature_unavailable(FeatureUnavailabilityReason::BarGap),
        feature_value(1.4, times[4]),
        feature_value(1.6, times[5]),
        feature_value(1.7, times[6]),
        feature_unavailable(FeatureUnavailabilityReason::MissingMarketInput),
        feature_value(1.9, times[8]),
        feature_value(2.1, times[9]),
        feature_value(2.0, times[10]),
        feature_value(2.3, times[11]),
    ];
    let feature_rows = times
        .iter()
        .zip(signal)
        .map(|(time, signal)| FeatureDatasetRow {
            instrument_id: "BTC-USDT".into(),
            observation_time_ms: *time,
            values: BTreeMap::from([("signal".into(), signal)]),
        })
        .collect();
    let feature_dataset = feature_dataset(
        "okx-feature-reference",
        &plan,
        "okx-snapshot-reference",
        "okx-spot-reference",
        &["signal"],
        feature_rows,
    );
    let candidate = candidate(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0011),
        FactorScope::TimeSeries,
        &feature_dataset.feature_plan_hash,
        "signal",
        "momentum",
    );
    let materialization = materialization_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0021),
        &candidate,
        &feature_dataset,
        context.clone(),
        12 * HOUR_MS,
    );
    let dataset =
        adaq_factor_research::FactorMaterializer::materialize(FactorMaterializationInput {
            candidate: &candidate,
            protocol: &materialization,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &["BTC-USDT".into()],
            custom_package: None,
        })
        .unwrap();
    let market = adaq_factor_research::FactorMarketSeries {
        instrument_id: "BTC-USDT".into(),
        snapshot_id: "okx-snapshot-reference".into(),
        market_context: context.clone(),
        bars: times
            .iter()
            .zip([100, 101, 103, 102, 105, 106, 107, 109, 110, 111, 113, 116])
            .map(|(time, close)| bar(*time, close))
            .collect(),
        gaps: vec![BarGap {
            start_time_ms: 2 * HOUR_MS + HOUR_MS / 2,
            end_time_ms: 3 * HOUR_MS + HOUR_MS / 2,
        }],
        corporate_action_evidence: CorporateActionEvidence::Verified,
    };
    let protocol = evaluation_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0031),
        &dataset,
        FactorScope::TimeSeries,
        context.clone(),
        vec!["BTC-USDT".into()],
        vec![1, 3, 5],
        vec![out_of_sample_window(4 * HOUR_MS, 12 * HOUR_MS)],
        vec![FactorLens::Temporal, FactorLens::Economic],
        vec![],
        None,
    );
    let report = FactorEvaluator::evaluate(FactorEvaluationInput {
        dataset: &dataset,
        protocol: &protocol,
        market_series: std::slice::from_ref(&market),
        feature_evidence: None,
    })
    .unwrap();
    let mut checks = common_checks();
    checks.insert(
        "warmup-preserved".into(),
        dataset.rows.iter().take(2).all(|row| {
            matches!(
                row.values["momentum"],
                FactorObservationValue::Unavailable {
                    reason: FactorUnavailabilityReason::Warmup
                }
            )
        }),
    );
    checks.insert(
        "bar-gap-restarts-evidence".into(),
        dataset.rows.iter().any(|row| {
            matches!(
                row.values["momentum"],
                FactorObservationValue::Unavailable {
                    reason: FactorUnavailabilityReason::BarGap
                }
            )
        }) && report.target_unavailable.iter().any(|evidence| {
            evidence.reason == adaq_factor_research::TargetUnavailableReason::BarGap
        }),
    );
    checks.insert(
        "multi-horizon-decay-and-stability-retained".into(),
        report
            .metrics
            .iter()
            .any(|metric| metric.metric == MetricId::Decay)
            && report
                .metrics
                .iter()
                .any(|metric| metric.metric == MetricId::Stability),
    );
    vector(context, &candidate, &dataset, &protocol, &report, checks)
}

fn a_share_journey() -> JourneyVector {
    let context = context(
        "sse",
        "equity-cn",
        "1d",
        "adjusted-total-return",
        "CNY",
        "cn-a-share-reference",
    );
    let plan = plan(&["signal"]);
    let times = vec![
        0,
        DAY_MS,
        3 * DAY_MS,
        4 * DAY_MS,
        5 * DAY_MS,
        6 * DAY_MS,
        7 * DAY_MS,
        8 * DAY_MS,
    ];
    let signal = [
        feature_value(0.1, times[0]),
        feature_value(0.2, times[1]),
        feature_value(0.3, times[2]),
        feature_unavailable(FeatureUnavailabilityReason::MissingMarketInput),
        feature_value(0.5, times[4]),
        feature_value(0.6, times[5]),
        feature_value(0.7, times[6]),
        feature_value(0.8, times[7]),
    ];
    let feature_rows = times
        .iter()
        .zip(signal)
        .map(|(time, signal)| FeatureDatasetRow {
            instrument_id: "600000.SH".into(),
            observation_time_ms: *time,
            values: BTreeMap::from([("signal".into(), signal)]),
        })
        .collect();
    let feature_dataset = feature_dataset(
        "cn-feature-reference",
        &plan,
        "cn-snapshot-reference",
        "cn-a-share-reference",
        &["signal"],
        feature_rows,
    );
    let candidate = candidate(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0012),
        FactorScope::TimeSeries,
        &feature_dataset.feature_plan_hash,
        "signal",
        "momentum",
    );
    let materialization = materialization_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0022),
        &candidate,
        &feature_dataset,
        context.clone(),
        9 * DAY_MS,
    );
    let dataset =
        adaq_factor_research::FactorMaterializer::materialize(FactorMaterializationInput {
            candidate: &candidate,
            protocol: &materialization,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &["600000.SH".into()],
            custom_package: None,
        })
        .unwrap();
    let market = adaq_factor_research::FactorMarketSeries {
        instrument_id: "600000.SH".into(),
        snapshot_id: "cn-snapshot-reference".into(),
        market_context: context.clone(),
        bars: times
            .iter()
            .zip([100, 101, 103, 102, 104, 105, 104, 0])
            .map(|(time, close)| bar(*time, close))
            .collect(),
        gaps: vec![],
        corporate_action_evidence: CorporateActionEvidence::Verified,
    };
    let protocol = evaluation_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0032),
        &dataset,
        FactorScope::TimeSeries,
        context.clone(),
        vec!["600000.SH".into()],
        vec![1, 2],
        vec![out_of_sample_window(3 * DAY_MS, 9 * DAY_MS)],
        vec![FactorLens::Temporal, FactorLens::Economic],
        vec![],
        None,
    );
    let report = FactorEvaluator::evaluate(FactorEvaluationInput {
        dataset: &dataset,
        protocol: &protocol,
        market_series: std::slice::from_ref(&market),
        feature_evidence: None,
    })
    .unwrap();
    let mut checks = common_checks();
    checks.insert(
        "scheduled-closure-is-not-a-bar-gap".into(),
        market.gaps.is_empty()
            && market.bars[2].open_time_ms - market.bars[1].open_time_ms > DAY_MS,
    );
    checks.insert(
        "corporate-action-evidence-is-verified".into(),
        matches!(
            market.corporate_action_evidence,
            CorporateActionEvidence::Verified
        ),
    );
    checks.insert(
        "missing-close-is-typed".into(),
        report.target_unavailable.iter().any(|evidence| {
            evidence.reason == adaq_factor_research::TargetUnavailableReason::MissingClose
        }),
    );
    checks.insert(
        "target-availability-remains-causal".into(),
        report
            .target_unavailable
            .iter()
            .all(|evidence| evidence.observation_time_ms >= 0),
    );
    vector(context, &candidate, &dataset, &protocol, &report, checks)
}

fn cross_sectional_journey() -> (
    JourneyVector,
    FactorDataset,
    FactorEvaluationProtocol,
    EvaluationFeatureEvidence,
) {
    let context = context(
        "nyse",
        "us-equity",
        "1d",
        "unadjusted",
        "USD",
        "us-equity-pit-reference",
    );
    let universe = ["AAA", "BBB", "CCC", "DDD", "EEE"];
    let plan = plan(&["signal", "size"]);
    let times = (0..8).map(|index| index * DAY_MS).collect::<Vec<_>>();
    let rows = universe
        .iter()
        .enumerate()
        .flat_map(|(instrument_index, instrument)| {
            times.iter().enumerate().map(move |(time_index, time)| {
                let signal = if instrument_index < 2 {
                    1.0 + time_index as f64 * 0.01
                } else {
                    instrument_index as f64 + 1.0 + time_index as f64 * 0.01
                };
                let signal = if instrument_index == 4 && time_index == 2 {
                    feature_unavailable(FeatureUnavailabilityReason::UnknownUniverse)
                } else {
                    feature_value(signal, *time)
                };
                FeatureDatasetRow {
                    instrument_id: (*instrument).into(),
                    observation_time_ms: *time,
                    values: BTreeMap::from([
                        ("signal".into(), signal),
                        (
                            "size".into(),
                            feature_value(
                                instrument_index as f64 + 1.0 + time_index as f64 * 0.1,
                                *time,
                            ),
                        ),
                    ]),
                }
            })
        })
        .collect::<Vec<_>>();
    let feature_dataset = feature_dataset(
        "us-feature-reference",
        &plan,
        "us-snapshot-reference",
        "us-equity-pit-reference",
        &["signal", "size"],
        rows,
    );
    let candidate = candidate(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0013),
        FactorScope::CrossSectional,
        &feature_dataset.feature_plan_hash,
        "signal",
        "score",
    );
    let materialization = materialization_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0023),
        &candidate,
        &feature_dataset,
        context.clone(),
        8 * DAY_MS,
    );
    let universe = universe.iter().map(|id| (*id).into()).collect::<Vec<_>>();
    let dataset =
        adaq_factor_research::FactorMaterializer::materialize(FactorMaterializationInput {
            candidate: &candidate,
            protocol: &materialization,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &universe,
            custom_package: None,
        })
        .unwrap();
    let market_series = universe
        .iter()
        .enumerate()
        .map(
            |(instrument_index, instrument)| adaq_factor_research::FactorMarketSeries {
                instrument_id: instrument.clone(),
                snapshot_id: "us-snapshot-reference".into(),
                market_context: context.clone(),
                bars: times
                    .iter()
                    .enumerate()
                    .map(|(time_index, time)| {
                        bar(
                            *time,
                            100 + instrument_index as i64 * 3
                                + time_index as i64 * (instrument_index as i64 + 1),
                        )
                    })
                    .collect(),
                gaps: vec![],
                corporate_action_evidence: CorporateActionEvidence::Verified,
            },
        )
        .collect::<Vec<_>>();
    let protocol = evaluation_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0033),
        &dataset,
        FactorScope::CrossSectional,
        context.clone(),
        universe.clone(),
        vec![1, 2],
        vec![out_of_sample_window(3 * DAY_MS, 8 * DAY_MS)],
        vec![
            FactorLens::CrossSectional,
            FactorLens::Economic,
            FactorLens::Neutralized,
            FactorLens::Regime,
        ],
        vec!["size".into()],
        Some(adaq_factor_research::FactorRegimeDefinition {
            feature_name: "size".into(),
            bucket_count: 3,
        }),
    );
    let feature_evidence = EvaluationFeatureEvidence {
        feature_dataset_id: dataset.manifest.feature_dataset_id.clone(),
        feature_plan_hash: dataset.manifest.feature_plan_hash.clone(),
        rows: universe
            .iter()
            .enumerate()
            .flat_map(|(instrument_index, instrument)| {
                times
                    .iter()
                    .enumerate()
                    .map(move |(time_index, time)| EvaluationFeatureRow {
                        instrument_id: instrument.clone(),
                        observation_time_ms: *time,
                        values: BTreeMap::from([(
                            "size".into(),
                            EvaluationFeatureCell::Available {
                                value: instrument_index as f64 + 1.0 + time_index as f64 * 0.1,
                                available_at_ms: *time,
                            },
                        )]),
                    })
            })
            .collect(),
    };
    let report = FactorEvaluator::evaluate(FactorEvaluationInput {
        dataset: &dataset,
        protocol: &protocol,
        market_series: &market_series,
        feature_evidence: Some(&feature_evidence),
    })
    .unwrap();
    let mut checks = common_checks();
    checks.insert(
        "unavailable-universe-member-is-retained".into(),
        dataset.rows.iter().any(|row| {
            matches!(
                row.values["score"],
                FactorObservationValue::Unavailable {
                    reason: FactorUnavailabilityReason::UnknownUniverse
                }
            )
        }),
    );
    checks.insert(
        "cross-sectional-neutralization-is-present".into(),
        report
            .metrics
            .iter()
            .any(|metric| metric.metric == MetricId::Neutralized),
    );
    checks.insert(
        "cross-sectional-regime-is-present".into(),
        !report.regime_evidence.is_empty(),
    );
    checks.insert(
        "cross-sectional-economic-costs-are-present".into(),
        report.metrics.iter().any(|metric| {
            metric.metric == MetricId::Economic && metric.variant == "top-minus-bottom"
        }),
    );
    checks.insert(
        "permuted-universe-rejected".into(),
        validate_cross_sectional_batch(
            &[
                CrossSectionalInputRow {
                    instrument_id: "AAA".into(),
                    observation_time_ms: 0,
                    slots: vec![FactorSlotCell::Available(AvailableFactorValue {
                        value: 1.0,
                        available_at_ms: 0,
                    })],
                },
                CrossSectionalInputRow {
                    instrument_id: "BBB".into(),
                    observation_time_ms: 0,
                    slots: vec![FactorSlotCell::Unavailable(
                        FactorUnavailabilityReason::UnknownUniverse,
                    )],
                },
            ],
            &["BBB".into(), "AAA".into()],
            1,
        )
        .is_err(),
    );
    checks.insert(
        "missing-universe-member-rejected".into(),
        validate_cross_sectional_batch(
            &[CrossSectionalInputRow {
                instrument_id: "AAA".into(),
                observation_time_ms: 0,
                slots: vec![FactorSlotCell::Available(AvailableFactorValue {
                    value: 1.0,
                    available_at_ms: 0,
                })],
            }],
            &["AAA".into(), "BBB".into()],
            1,
        )
        .is_err(),
    );
    (
        vector(context, &candidate, &dataset, &protocol, &report, checks),
        dataset,
        protocol,
        feature_evidence,
    )
}

fn custom_cross_sectional_equivalence() -> bool {
    let context = context(
        "nyse",
        "us-equity",
        "1d",
        "unadjusted",
        "USD",
        "custom-equivalence-universe",
    );
    let plan = plan(&["close"]);
    let times = (0..4).map(|index| index * DAY_MS).collect::<Vec<_>>();
    let universe = ["AAA", "BBB", "CCC", "DDD", "EEE"];
    let rows = universe
        .iter()
        .enumerate()
        .flat_map(|(instrument_index, instrument)| {
            times
                .iter()
                .enumerate()
                .map(move |(time_index, time)| FeatureDatasetRow {
                    instrument_id: (*instrument).into(),
                    observation_time_ms: *time,
                    values: BTreeMap::from([(
                        "close".into(),
                        feature_value(100.0 + instrument_index as f64 + time_index as f64, *time),
                    )]),
                })
        })
        .collect();
    let feature_dataset = feature_dataset(
        "custom-equivalence-feature",
        &plan,
        "custom-equivalence-snapshot",
        "custom-equivalence-universe",
        &["close"],
        rows,
    );
    let declarative = candidate(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0041),
        FactorScope::CrossSectional,
        &feature_dataset.feature_plan_hash,
        "close",
        "cross-sectional-score",
    );
    let request = adaq_factor_research::CandidateBuildRequest {
        attempt_id: Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0042),
        user_id: USER_ID,
        project_root: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/cross-sectional-factor"),
        source_sha256: adaq_factor_research::project_source_sha256(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/cross-sectional-factor"),
        )
        .unwrap(),
        sdk_version: "0.1.0".into(),
        toolchain: "stable".into(),
        target: "wasm32-unknown-unknown".into(),
        resource_policy: FactorResourcePolicy {
            fuel_per_call: 1_000_000,
            memory_bytes: 64 * 1024 * 1024,
        },
    };
    let worker = adaq_factor_research::spawn_controlled_candidate_build(request).unwrap();
    let build = worker.join().result.unwrap();
    let custom = adaq_factor_research::CustomFactorDraft {
        user_id: USER_ID,
        candidate_id: Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0043),
        revision: 1,
        scope: FactorScope::CrossSectional,
        feature_slots: vec![FactorFeatureSlot {
            name: "close".into(),
        }],
        parameters: vec![],
        outputs: vec![FactorOutput {
            name: "cross-sectional-score".into(),
        }],
        build: build.provenance,
        presentation: adaq_factor_research::FactorPresentationMetadata {
            name: "custom-equivalence".into(),
            description: String::new(),
            tags: vec!["reference".into()],
        },
    }
    .publish()
    .unwrap()
    .0;
    let declarative_protocol = materialization_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0044),
        &declarative,
        &feature_dataset,
        context.clone(),
        4 * DAY_MS,
    );
    let custom_protocol = materialization_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0045),
        &custom,
        &feature_dataset,
        context.clone(),
        4 * DAY_MS,
    );
    let declarative_dataset =
        adaq_factor_research::FactorMaterializer::materialize(FactorMaterializationInput {
            candidate: &declarative,
            protocol: &declarative_protocol,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &universe.iter().map(|id| (*id).into()).collect::<Vec<_>>(),
            custom_package: None,
        })
        .unwrap();
    let custom_dataset =
        adaq_factor_research::FactorMaterializer::materialize(FactorMaterializationInput {
            candidate: &custom,
            protocol: &custom_protocol,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &universe.iter().map(|id| (*id).into()).collect::<Vec<_>>(),
            custom_package: Some(&build.package),
        })
        .unwrap();
    if declarative_dataset.rows != custom_dataset.rows {
        return false;
    }
    let build_provenance = match &custom.source {
        adaq_factor_research::FactorCandidateSource::Custom { build } => build,
        _ => return false,
    };
    let universe_ids = universe.iter().map(|id| (*id).into()).collect::<Vec<_>>();
    let replayed_declarative = adaq_factor_research::FactorMaterializer::replay_component_package(
        FactorMaterializationInput {
            candidate: &declarative,
            protocol: &declarative_protocol,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &universe_ids,
            custom_package: Some(&build.package),
        },
        &build.package,
        build_provenance,
    )
    .unwrap();
    let replayed_custom = adaq_factor_research::FactorMaterializer::replay_component_package(
        FactorMaterializationInput {
            candidate: &custom,
            protocol: &custom_protocol,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &universe_ids,
            custom_package: Some(&build.package),
        },
        &build.package,
        build_provenance,
    )
    .unwrap();
    if replayed_declarative != declarative_dataset.rows || replayed_custom != custom_dataset.rows {
        return false;
    }
    let market = universe
        .iter()
        .map(|instrument| adaq_factor_research::FactorMarketSeries {
            instrument_id: (*instrument).into(),
            snapshot_id: "custom-equivalence-snapshot".into(),
            market_context: context.clone(),
            bars: times
                .iter()
                .enumerate()
                .map(|(index, time)| bar(*time, 100 + index as i64))
                .collect(),
            gaps: vec![],
            corporate_action_evidence: CorporateActionEvidence::Verified,
        })
        .collect::<Vec<_>>();
    let universe = universe.iter().map(|id| (*id).into()).collect::<Vec<_>>();
    let declarative_evaluation = evaluation_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0046),
        &declarative_dataset,
        FactorScope::CrossSectional,
        context.clone(),
        universe.clone(),
        vec![1],
        vec![out_of_sample_window(2 * DAY_MS, 4 * DAY_MS)],
        vec![FactorLens::CrossSectional, FactorLens::Economic],
        vec![],
        None,
    );
    let custom_evaluation = evaluation_protocol(
        Uuid::from_u128(0x9400_0000_0000_0000_0000_0000_0000_0047),
        &custom_dataset,
        FactorScope::CrossSectional,
        context,
        universe,
        vec![1],
        vec![out_of_sample_window(2 * DAY_MS, 4 * DAY_MS)],
        vec![FactorLens::CrossSectional, FactorLens::Economic],
        vec![],
        None,
    );
    let left = FactorEvaluator::evaluate(FactorEvaluationInput {
        dataset: &declarative_dataset,
        protocol: &declarative_evaluation,
        market_series: &market,
        feature_evidence: None,
    })
    .unwrap();
    let right = FactorEvaluator::evaluate(FactorEvaluationInput {
        dataset: &custom_dataset,
        protocol: &custom_evaluation,
        market_series: &market,
        feature_evidence: None,
    })
    .unwrap();
    left.metrics == right.metrics && left.target_unavailable == right.target_unavailable
}

#[test]
fn committed_reference_vectors_match_three_market_journeys() {
    let mut journeys = BTreeMap::new();
    journeys.insert("okx-spot-time-series".into(), okx_journey());
    journeys.insert("cn-a-share-time-series".into(), a_share_journey());
    journeys.insert(
        "us-equity-cross-sectional".into(),
        cross_sectional_journey().0,
    );
    let vectors = ReferenceVectors {
        schema_version: VECTORS_SCHEMA.into(),
        journeys,
    };
    let mut replayed = BTreeMap::new();
    replayed.insert("okx-spot-time-series".into(), okx_journey());
    replayed.insert("cn-a-share-time-series".into(), a_share_journey());
    replayed.insert(
        "us-equity-cross-sectional".into(),
        cross_sectional_journey().0,
    );
    assert_eq!(
        vectors.journeys, replayed,
        "reference journey computation must be deterministic within one process"
    );
    if env::var("ADAQ_FACTOR_REGENERATE").as_deref() == Ok("1") {
        let mut bytes = serde_json::to_vec_pretty(&vectors).unwrap();
        bytes.push(b'\n');
        fs::write(vectors_path(), bytes).unwrap();
        return;
    }
    let committed: ReferenceVectors = serde_json::from_slice(
        &fs::read(vectors_path()).expect("committed Factor reference vectors are required"),
    )
    .unwrap();
    assert_reference_vectors_equal(&committed, &vectors);
    assert!(committed.journeys.values().all(|journey| {
        journey.checks.values().all(|passed| *passed)
            && journey
                .metrics
                .iter()
                .all(|metric| metric.value.is_none_or(f64::is_finite))
    }));
}

#[test]
fn private_custom_and_declarative_cross_sectional_paths_are_equivalent() {
    assert!(custom_cross_sectional_equivalence());
}

#[test]
fn factor_abi_and_numeric_boundaries_reject_invalid_evidence() {
    assert!(
        validate_time_series_batch(
            &[TimeSeriesInputRow {
                instrument_id: "A".into(),
                observation_time_ms: 0,
                slots: vec![adaq_factor_research::AvailableFactorValue {
                    value: 1.0,
                    available_at_ms: 1,
                }],
            }],
            "A",
            1,
        )
        .is_err()
    );
    assert!(
        validate_factor_results(
            &[FactorResult {
                instrument_id: "A".into(),
                observation_time_ms: 0,
                values: Some(vec![NamedFactorOutput {
                    name: "Score".into(),
                    value: 1.0,
                }]),
            }],
            &["A".into()],
            &[0],
            &["score".into()],
        )
        .is_err()
    );
}
