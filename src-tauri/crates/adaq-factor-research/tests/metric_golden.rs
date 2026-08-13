//! Independent literal vectors for the Factor Metric Catalog and evaluator.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use adaq_data_core::OhlcvBar;
use adaq_factor_research::{
    CorporateActionEvidence, EconomicAssumptions, EvaluationFeatureCell, EvaluationFeatureEvidence,
    EvaluationFeatureRow, EvaluationWindow, FactorDataset, FactorDatasetManifest, FactorDatasetRow,
    FactorEvaluationInput, FactorEvaluationProtocol, FactorEvaluationProtocolDraft,
    FactorEvaluator, FactorLens, FactorMarketContext, FactorObservationValue, FactorOrientation,
    FactorScope, FactorTarget, MetricId, MetricObservation, MetricUndefinedReason,
    ObservationRange, ResearchEngineProvenance, content_hash, holm_bonferroni,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CATALOG_FILE: &str = "fixtures/factor-metric-catalog.json";
const GOLDEN_FILE: &str = "fixtures/factor-metric-golden.json";
const CATALOG_SCHEMA: &str = "adaq-factor-metric-catalog-reference@1.0.0";
const GOLDEN_SCHEMA: &str = "adaq-factor-metric-golden@1.0.0";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogReference {
    schema_version: String,
    catalog: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Golden {
    schema_version: String,
    average_rank_ties: Vec<f64>,
    rank_ic: f64,
    constant: UndefinedVector,
    singular_matrix: UndefinedVector,
    raw_p_value: ScalarVector,
    holm: HolmVector,
    costs: CostVector,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UndefinedVector {
    reason: MetricUndefinedReason,
    sample_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScalarVector {
    value: f64,
    sample_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HolmVector {
    family_size: usize,
    adjusted_p_values: Vec<f64>,
    sample_counts: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CostVector {
    top_only: f64,
    top_minus_bottom: f64,
}

fn path(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(file)
}

fn engine() -> ResearchEngineProvenance {
    ResearchEngineProvenance {
        engine_id: "adaq-native-factor-golden".into(),
        engine_version: "1.0.0".into(),
        adapter: "native".into(),
        target_triple: "fixture".into(),
        build_id: "golden".into(),
        environment: BTreeMap::new(),
        parameters: BTreeMap::new(),
        input_identities: vec!["golden-input".into()],
    }
}

fn context(universe: &str) -> FactorMarketContext {
    FactorMarketContext {
        venue: "golden".into(),
        asset_class: "fixture".into(),
        bar_interval: "1d".into(),
        price_basis: "unadjusted".into(),
        valuation_currency: "USD".into(),
        point_in_time_universe_id: universe.into(),
    }
}

fn bar(time_ms: i64, close: i64) -> OhlcvBar {
    bar_decimal(time_ms, Decimal::from(close))
}

fn bar_decimal(time_ms: i64, close: Decimal) -> OhlcvBar {
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

fn row(instrument: &str, time_ms: i64, value: f64) -> FactorDatasetRow {
    FactorDatasetRow {
        instrument_id: instrument.into(),
        observation_time_ms: time_ms,
        values: BTreeMap::from([(
            "score".into(),
            FactorObservationValue::Available {
                value,
                available_at_ms: time_ms,
            },
        )]),
    }
}

fn dataset(
    scope: FactorScope,
    universe_id: &str,
    instruments: &[&str],
    rows: Vec<FactorDatasetRow>,
) -> FactorDataset {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Payload<'a> {
        output_names: &'a [String],
        rows: &'a [FactorDatasetRow],
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Identity<'a> {
        schema_version: &'a str,
        protocol_hash: &'a str,
        candidate_hash: &'a str,
        scope: FactorScope,
        feature_dataset_id: &'a str,
        feature_plan_hash: &'a str,
        market_data_snapshot_id: &'a str,
        point_in_time_universe_id: &'a str,
        market_context: &'a FactorMarketContext,
        output_names: &'a [String],
        observation_count: u64,
        payload_sha256: &'a str,
        engine_identity: &'a ResearchEngineProvenance,
    }
    let output_names = vec!["score".into()];
    let payload = serde_json::to_vec(&Payload {
        output_names: &output_names,
        rows: &rows,
    })
    .unwrap();
    let payload_sha256 = adaq_feature_engine::sha256(&payload);
    let protocol_hash = "a".repeat(64);
    let candidate_hash = "b".repeat(64);
    let feature_plan_hash = "c".repeat(64);
    let market_context = context(universe_id);
    let feature_dataset_id = "golden-feature-dataset";
    let market_data_snapshot_id = "golden-snapshot";
    let engine_identity = engine();
    let mut manifest = FactorDatasetManifest {
        schema_version: adaq_factor_research::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
        dataset_id: String::new(),
        protocol_hash: protocol_hash.clone(),
        candidate_hash: candidate_hash.clone(),
        scope,
        feature_dataset_id: feature_dataset_id.into(),
        feature_plan_hash: feature_plan_hash.clone(),
        market_data_snapshot_id: market_data_snapshot_id.into(),
        point_in_time_universe_id: universe_id.into(),
        market_context,
        output_names,
        observation_count: rows.len() as u64,
        payload_sha256,
        engine_identity,
    };
    manifest.dataset_id = content_hash(&Identity {
        schema_version: &manifest.schema_version,
        protocol_hash: &manifest.protocol_hash,
        candidate_hash: &manifest.candidate_hash,
        scope: manifest.scope,
        feature_dataset_id: &manifest.feature_dataset_id,
        feature_plan_hash: &manifest.feature_plan_hash,
        market_data_snapshot_id: &manifest.market_data_snapshot_id,
        point_in_time_universe_id: &manifest.point_in_time_universe_id,
        market_context: &manifest.market_context,
        output_names: &manifest.output_names,
        observation_count: manifest.observation_count,
        payload_sha256: &manifest.payload_sha256,
        engine_identity: &manifest.engine_identity,
    })
    .unwrap();
    let dataset = FactorDataset { manifest, rows };
    assert_eq!(dataset.manifest.point_in_time_universe_id, universe_id);
    assert_eq!(dataset.manifest.output_names, ["score"]);
    assert_eq!(
        dataset.manifest.observation_count as usize,
        dataset.rows.len()
    );
    assert_eq!(
        dataset.manifest.market_context.point_in_time_universe_id,
        universe_id
    );
    assert_eq!(dataset.manifest.market_data_snapshot_id, "golden-snapshot");
    assert_eq!(dataset.manifest.feature_dataset_id, feature_dataset_id);
    assert_eq!(dataset.manifest.candidate_hash, candidate_hash);
    assert_eq!(dataset.manifest.feature_plan_hash, feature_plan_hash);
    assert_eq!(dataset.manifest.protocol_hash, protocol_hash);
    assert_eq!(dataset.manifest.engine_identity, engine());
    assert!(instruments.iter().all(|instrument| {
        dataset
            .rows
            .iter()
            .any(|row| row.instrument_id == *instrument)
    }));
    dataset
}

fn window(start: i64, end: i64) -> EvaluationWindow {
    EvaluationWindow {
        fold_id: "golden-fold".into(),
        selection: ObservationRange {
            start_time_ms: -100,
            end_time_ms: start,
        },
        evaluation: ObservationRange {
            start_time_ms: start,
            end_time_ms: end,
        },
        training: Some(ObservationRange {
            start_time_ms: -100,
            end_time_ms: -75,
        }),
        fitting: Some(ObservationRange {
            start_time_ms: -100,
            end_time_ms: -75,
        }),
        normalization: Some(ObservationRange {
            start_time_ms: -100,
            end_time_ms: -75,
        }),
        target_construction: Some(ObservationRange {
            start_time_ms: -100,
            end_time_ms: -75,
        }),
    }
}

fn protocol(
    dataset: &FactorDataset,
    scope: FactorScope,
    instruments: Vec<String>,
    lenses: Vec<FactorLens>,
    nuisance_feature_names: Vec<String>,
) -> FactorEvaluationProtocol {
    FactorEvaluationProtocol::freeze(FactorEvaluationProtocolDraft {
        protocol_id: Uuid::from_u128(0x9500_0000_0000_0000_0000_0000_0000_0001),
        user_id: Uuid::from_u128(0x9500_0000_0000_0000_0000_0000_0000_0002),
        factor_dataset_id: dataset.manifest.dataset_id.clone(),
        feature_dataset_id: dataset.manifest.feature_dataset_id.clone(),
        feature_plan_hash: dataset.manifest.feature_plan_hash.clone(),
        market_data_snapshot_id: dataset.manifest.market_data_snapshot_id.clone(),
        point_in_time_universe_id: dataset.manifest.point_in_time_universe_id.clone(),
        point_in_time_universe: instruments,
        output_name: "score".into(),
        scope,
        target: FactorTarget::FutureCloseReturn,
        horizon_bars: vec![1],
        market_context: dataset.manifest.market_context.clone(),
        engine_identity: engine(),
        orientation: FactorOrientation::Positive,
        windows: vec![window(0, 5)],
        purge_bars: 0,
        embargo_bars: 0,
        lenses,
        nuisance_feature_names,
        regime: None,
        economic: EconomicAssumptions {
            rebalance_every_bars: 1,
            fee_bps: 10.0,
            slippage_bps: 5.0,
            long_short: true,
        },
        family_id: Uuid::from_u128(0x9500_0000_0000_0000_0000_0000_0000_0003),
        trial_id: Uuid::from_u128(0x9500_0000_0000_0000_0000_0000_0000_0004),
        seed: 95,
    })
    .unwrap()
}

fn series(instrument: &str, closes: &[i64]) -> adaq_factor_research::FactorMarketSeries {
    adaq_factor_research::FactorMarketSeries {
        instrument_id: instrument.into(),
        snapshot_id: "golden-snapshot".into(),
        market_context: context("golden-universe"),
        bars: closes
            .iter()
            .enumerate()
            .map(|(index, close)| bar(index as i64, *close))
            .collect(),
        gaps: vec![],
        corporate_action_evidence: CorporateActionEvidence::Verified,
    }
}

fn decimal_series(instrument: &str, closes: &[&str]) -> adaq_factor_research::FactorMarketSeries {
    adaq_factor_research::FactorMarketSeries {
        instrument_id: instrument.into(),
        snapshot_id: "golden-snapshot".into(),
        market_context: context("golden-universe"),
        bars: closes
            .iter()
            .enumerate()
            .map(|(index, close)| bar_decimal(index as i64, close.parse().expect("golden decimal")))
            .collect(),
        gaps: vec![],
        corporate_action_evidence: CorporateActionEvidence::Verified,
    }
}

fn metric<'a>(
    report: &'a adaq_factor_research::FactorEvaluationReport,
    id: MetricId,
    variant: &str,
) -> &'a MetricObservation {
    &report
        .metrics
        .iter()
        .find(|record| record.metric == id && record.variant == variant)
        .unwrap_or_else(|| panic!("missing golden metric {id:?}/{variant}"))
        .observation
}

fn value(observation: &MetricObservation) -> f64 {
    observation
        .value()
        .expect("golden metric should be available")
}

fn undefined(observation: &MetricObservation) -> UndefinedVector {
    match observation {
        MetricObservation::Unavailable {
            reason,
            sample_count,
        } => UndefinedVector {
            reason: *reason,
            sample_count: *sample_count,
        },
        MetricObservation::Available { .. } => panic!("golden metric should be unavailable"),
    }
}

#[test]
fn committed_metric_catalog_reference_is_current() {
    let catalog = adaq_factor_research::FactorMetricCatalog::initial();
    catalog.validate().unwrap();
    let current = serde_json::from_slice::<serde_json::Value>(&catalog.to_json().unwrap()).unwrap();
    let expected: CatalogReference = serde_json::from_slice(
        &fs::read(path(CATALOG_FILE))
            .expect("committed Factor Metric Catalog reference is required"),
    )
    .unwrap();
    assert_eq!(expected.schema_version, CATALOG_SCHEMA);
    assert_eq!(expected.catalog, current);
}

#[test]
#[ignore]
fn regenerate_factor_metric_catalog() {
    let catalog = adaq_factor_research::FactorMetricCatalog::initial();
    catalog.validate().unwrap();
    let reference = CatalogReference {
        schema_version: CATALOG_SCHEMA.into(),
        catalog: serde_json::from_slice(&catalog.to_json().unwrap()).unwrap(),
    };
    let mut bytes = serde_json::to_vec_pretty(&reference).unwrap();
    bytes.push(b'\n');
    fs::write(path(CATALOG_FILE), bytes).unwrap();
}

#[test]
fn independent_metric_golden_vectors_match_evaluator_and_holm() {
    let golden: Golden = serde_json::from_slice(
        &fs::read(path(GOLDEN_FILE)).expect("committed Factor metric golden vectors are required"),
    )
    .unwrap();
    assert_eq!(golden.schema_version, GOLDEN_SCHEMA);
    assert_eq!(golden.average_rank_ties, [1.5, 1.5, 3.0, 4.0, 5.0]);

    let tied_rows = [("A", 1.0), ("B", 1.0), ("C", 3.0), ("D", 4.0), ("E", 5.0)]
        .into_iter()
        .flat_map(|(instrument, score)| (0..5).map(move |time| row(instrument, time, score)))
        .collect();
    let tied_dataset = dataset(
        FactorScope::CrossSectional,
        "golden-universe",
        &["A", "B", "C", "D", "E"],
        tied_rows,
    );
    let tied_protocol = protocol(
        &tied_dataset,
        FactorScope::CrossSectional,
        ["A", "B", "C", "D", "E"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        vec![FactorLens::CrossSectional, FactorLens::Economic],
        vec![],
    );
    let mut oversized = tied_protocol.clone();
    oversized.horizon_bars = vec![1; adaq_factor_research::MAX_FACTOR_EVALUATION_HORIZONS + 1];
    assert!(oversized.validate().is_err());
    oversized = tied_protocol.clone();
    oversized.windows = vec![window(0, 5); adaq_factor_research::MAX_FACTOR_EVALUATION_FOLDS + 1];
    assert!(oversized.validate().is_err());
    oversized = tied_protocol.clone();
    oversized.lenses = vec![FactorLens::CrossSectional, FactorLens::CrossSectional];
    assert!(oversized.validate().is_err());
    oversized = tied_protocol.clone();
    oversized.nuisance_feature_names = (0..adaq_factor_research::MAX_FACTOR_NUISANCE_FEATURES + 1)
        .map(|index| format!("nuisance-{index}"))
        .collect();
    assert!(oversized.validate().is_err());
    assert!(adaq_factor_research::checked_product([u64::MAX, 2]).is_err());
    let market = vec![
        decimal_series("A", &["100", "100", "100", "100", "100", "100"]),
        decimal_series(
            "B",
            &[
                "100",
                "101",
                "102.01",
                "103.0301",
                "104.060401",
                "105.10100501",
            ],
        ),
        decimal_series(
            "C",
            &[
                "100",
                "102",
                "104.04",
                "106.1208",
                "108.243216",
                "110.40808032",
            ],
        ),
        decimal_series(
            "D",
            &[
                "100",
                "103",
                "106.09",
                "109.2727",
                "112.550881",
                "115.92740743",
            ],
        ),
        decimal_series(
            "E",
            &[
                "100",
                "104",
                "108.16",
                "112.4864",
                "116.985856",
                "121.66529024",
            ],
        ),
    ];
    let tied_report = FactorEvaluator::evaluate(FactorEvaluationInput {
        dataset: &tied_dataset,
        protocol: &tied_protocol,
        market_series: &market,
        feature_evidence: None,
    })
    .unwrap();
    let rank_ic = metric(&tied_report, MetricId::RankIc, "all");
    assert!((value(rank_ic) - golden.rank_ic).abs() < 1e-12);
    let economic_top = value(metric(&tied_report, MetricId::Economic, "top-only"));
    let economic_spread = value(metric(&tied_report, MetricId::Economic, "top-minus-bottom"));
    assert!((economic_top - golden.costs.top_only).abs() < 1e-12);
    assert!((economic_spread - golden.costs.top_minus_bottom).abs() < 1e-12);

    let constant_dataset = dataset(
        FactorScope::TimeSeries,
        "golden-universe",
        &["A"],
        vec![row("A", 0, 1.0), row("A", 1, 1.0), row("A", 2, 1.0)],
    );
    let constant_protocol = protocol(
        &constant_dataset,
        FactorScope::TimeSeries,
        vec!["A".into()],
        vec![FactorLens::Temporal, FactorLens::Economic],
        vec![],
    );
    let constant_report = FactorEvaluator::evaluate(FactorEvaluationInput {
        dataset: &constant_dataset,
        protocol: &constant_protocol,
        market_series: &[series("A", &[100, 101, 102, 103])],
        feature_evidence: None,
    })
    .unwrap();
    assert_eq!(
        undefined(metric(&constant_report, MetricId::Ic, "A")),
        golden.constant
    );

    let feature_evidence = EvaluationFeatureEvidence {
        feature_dataset_id: tied_dataset.manifest.feature_dataset_id.clone(),
        feature_plan_hash: tied_dataset.manifest.feature_plan_hash.clone(),
        rows: ["A", "B", "C", "D", "E"]
            .into_iter()
            .flat_map(|instrument| {
                (0..5).map(move |time| EvaluationFeatureRow {
                    instrument_id: instrument.into(),
                    observation_time_ms: time,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 1.0,
                            available_at_ms: time,
                        },
                    )]),
                })
            })
            .collect(),
    };
    let mut singular_protocol = tied_protocol.clone();
    singular_protocol.lenses.push(FactorLens::Neutralized);
    singular_protocol.nuisance_feature_names = vec!["size".into()];
    singular_protocol.protocol_hash = String::new();
    singular_protocol = FactorEvaluationProtocol::freeze(FactorEvaluationProtocolDraft {
        protocol_id: singular_protocol.protocol_id,
        user_id: singular_protocol.user_id,
        factor_dataset_id: singular_protocol.factor_dataset_id,
        feature_dataset_id: singular_protocol.feature_dataset_id,
        feature_plan_hash: singular_protocol.feature_plan_hash,
        market_data_snapshot_id: singular_protocol.market_data_snapshot_id,
        point_in_time_universe_id: singular_protocol.point_in_time_universe_id,
        point_in_time_universe: singular_protocol.point_in_time_universe,
        output_name: singular_protocol.output_name,
        scope: singular_protocol.scope,
        target: singular_protocol.target,
        horizon_bars: singular_protocol.horizon_bars,
        market_context: singular_protocol.market_context,
        engine_identity: singular_protocol.engine_identity,
        orientation: singular_protocol.orientation,
        windows: singular_protocol.windows,
        purge_bars: singular_protocol.purge_bars,
        embargo_bars: singular_protocol.embargo_bars,
        lenses: vec![
            FactorLens::CrossSectional,
            FactorLens::Economic,
            FactorLens::Neutralized,
        ],
        nuisance_feature_names: vec!["size".into()],
        regime: None,
        economic: singular_protocol.economic,
        family_id: singular_protocol.family_id,
        trial_id: singular_protocol.trial_id,
        seed: singular_protocol.seed,
    })
    .unwrap();
    let singular_report = FactorEvaluator::evaluate(FactorEvaluationInput {
        dataset: &tied_dataset,
        protocol: &singular_protocol,
        market_series: &market,
        feature_evidence: Some(&feature_evidence),
    })
    .unwrap();
    assert_eq!(
        undefined(metric(&singular_report, MetricId::Neutralized, "all")),
        golden.singular_matrix
    );

    let raw =
        MetricObservation::available(golden.raw_p_value.value, golden.raw_p_value.sample_count)
            .unwrap();
    assert_eq!(
        raw,
        MetricObservation::Available {
            value: golden.raw_p_value.value,
            sample_count: golden.raw_p_value.sample_count,
        }
    );
    let trial_ids = [
        Uuid::from_u128(0x9500_0000_0000_0000_0000_0000_0000_0011),
        Uuid::from_u128(0x9500_0000_0000_0000_0000_0000_0000_0012),
        Uuid::from_u128(0x9500_0000_0000_0000_0000_0000_0000_0013),
    ];
    let correction = holm_bonferroni(&[
        (trial_ids[0], Some(raw.clone())),
        (
            trial_ids[1],
            Some(MetricObservation::available(0.02, 43).unwrap()),
        ),
        (trial_ids[2], None),
    ])
    .unwrap();
    assert_eq!(correction.family_size, golden.holm.family_size);
    let adjusted = trial_ids
        .iter()
        .map(|id| correction.adjusted_p_values[id].value().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(adjusted, golden.holm.adjusted_p_values);
    let counts = trial_ids
        .iter()
        .map(|id| match &correction.adjusted_p_values[id] {
            MetricObservation::Available { sample_count, .. }
            | MetricObservation::Unavailable { sample_count, .. } => *sample_count,
        })
        .collect::<Vec<_>>();
    assert_eq!(counts, golden.holm.sample_counts);
}
