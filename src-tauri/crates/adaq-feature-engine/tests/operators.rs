use std::collections::{BTreeMap, HashMap};

use adaq_data_core::{
    BarInterval,
    market::{
        DayEvidence, InstrumentId, PriceBasis, SessionPhase, TradingCalendarSnapshot,
        TradingSession, Venue, VenueKind,
    },
};
use adaq_feature_engine::{
    CorporateAction, DefinitionDraft, FeatureDefinition, FeatureDependencyInput, FeatureEngine,
    FeatureEngineIdentity, FeatureEvaluationInput, FeatureFactor, FeatureInput, FeatureInputEvent,
    FeatureMarketBar, FeatureMarketContext, FeatureNode, FeatureObservation,
    FeatureObservationValue, FeatureOperator, FeatureOutput, FeaturePlan, FeaturePlanDraft,
    FeatureScope, FeatureSlot, FeatureSource, FeatureUnavailabilityReason, MarketField,
    PointInTimeInstrumentUniverse, UniverseEvidenceState,
};
use chrono::NaiveTime;
use serde_json::json;
use uuid::Uuid;

fn identity() -> FeatureEngineIdentity {
    FeatureEngineIdentity::for_tests()
}

fn plan(nodes: Vec<FeatureNode>, outputs: &[(&str, &str)]) -> FeaturePlan {
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes,
        outputs: outputs
            .iter()
            .map(|(name, node_id)| FeatureOutput {
                name: (*name).into(),
                node_id: (*node_id).into(),
            })
            .collect(),
    })
    .unwrap();
    FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap()
}

fn node(
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

fn bar(time: i64, close: &str, base_volume: &str, quote_volume: &str) -> FeatureMarketBar {
    FeatureMarketBar::complete(time, close, close, close, close, base_volume, quote_volume).unwrap()
}

fn event(time: i64, close: &str, base_volume: &str, quote_volume: &str) -> FeatureInputEvent {
    FeatureInputEvent::observation(FeatureEvaluationInput::new(
        "BTC-USD",
        time,
        time,
        bar(time, close, base_volume, quote_volume),
    ))
}

fn value(observation: &FeatureObservation) -> Option<f64> {
    match observation.value {
        FeatureObservationValue::Available { value, .. } => Some(value),
        FeatureObservationValue::Unavailable { .. } => None,
    }
}

fn reason(observation: &FeatureObservation) -> Option<FeatureUnavailabilityReason> {
    match observation.value {
        FeatureObservationValue::Available { .. } => None,
        FeatureObservationValue::Unavailable { reason } => Some(reason),
    }
}

fn assert_close(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("expected an available feature");
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

fn cross_sectional_context() -> FeatureMarketContext {
    FeatureMarketContext::new(
        Venue::us_equity("iex").unwrap(),
        VenueKind::UsEquity,
        BarInterval::OneDay,
        PriceBasis::Unadjusted,
        "USD",
    )
    .unwrap()
}

fn cross_sectional_universe(
    time: i64,
    members: &[&str],
    evidence_state: UniverseEvidenceState,
) -> PointInTimeInstrumentUniverse {
    let context = cross_sectional_context();
    PointInTimeInstrumentUniverse::new(
        "universe-1",
        time,
        members.iter().map(|member| (*member).to_owned()).collect(),
        context,
        evidence_state,
    )
    .unwrap()
}

fn cross_sectional_batch(
    time: i64,
    universe: PointInTimeInstrumentUniverse,
    values: &[(&str, &str)],
) -> FeatureInputEvent {
    let context = cross_sectional_context();
    FeatureInputEvent::cross_sectional_batch(
        time,
        universe,
        values
            .iter()
            .map(|(instrument_id, close)| {
                FeatureEvaluationInput::new(*instrument_id, time, time, bar(time, close, "1", "1"))
                    .with_market_context(context.clone())
            })
            .collect(),
    )
}

fn cross_sectional_plan(nodes: Vec<FeatureNode>, outputs: &[(&str, &str)]) -> FeaturePlan {
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::CrossSectional,
        nodes,
        outputs: outputs
            .iter()
            .map(|(name, node_id)| FeatureOutput {
                name: (*name).into(),
                node_id: (*node_id).into(),
            })
            .collect(),
    })
    .unwrap();
    FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap()
}

fn cross_sectional_node(
    id: &str,
    operator: FeatureOperator,
    parameters: BTreeMap<String, serde_json::Value>,
) -> FeatureNode {
    FeatureNode {
        id: id.into(),
        operator,
        scope: FeatureScope::CrossSectional,
        inputs: vec![FeatureInput::Market {
            field: MarketField::Close,
        }],
        parameters,
        warmup_bars: 0,
    }
}

#[test]
fn dependency_slots_share_batch_and_stateful_evaluation() {
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        factors: vec![adaq_feature_engine::FeatureFactor {
            alias: "momentum".into(),
            parameters: Vec::new(),
            output_names: vec!["score".into()],
            warmup_bars: 1,
        }],
        slots: vec![adaq_feature_engine::FeatureSlot {
            name: "factor-score".into(),
            source: adaq_feature_engine::FeatureSource::External {
                dependency_alias: "momentum".into(),
                output: "score".into(),
            },
            warmup_bars: 0,
        }],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    let first = FeatureInputEvent::observation(
        FeatureEvaluationInput::new("BTC-USD", 1, 1, bar(1, "10", "1", "1")).with_dependency(
            FeatureDependencyInput::external("momentum", "score", None, 1),
        ),
    );
    let second = FeatureInputEvent::observation(
        FeatureEvaluationInput::new("BTC-USD", 2, 2, bar(2, "11", "1", "1")).with_dependency(
            FeatureDependencyInput::external("momentum", "score", Some(2.5), 2),
        ),
    );
    let missing = FeatureInputEvent::observation(
        FeatureEvaluationInput::new("BTC-USD", 3, 3, bar(3, "12", "1", "1")).with_dependency(
            FeatureDependencyInput::external("momentum", "score", None, 3),
        ),
    );
    let mut batch = FeatureEngine::new(identity())
        .evaluator(plan.clone())
        .unwrap();
    let expected = batch
        .evaluate_batch(&[first.clone(), second.clone(), missing.clone()])
        .unwrap();

    let mut stateful = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let mut actual = stateful.observe(first).unwrap();
    actual.extend(stateful.observe(second).unwrap());
    actual.extend(stateful.observe(missing).unwrap());

    assert_eq!(actual, expected);
    assert_eq!(
        reason(&actual[0]),
        Some(FeatureUnavailabilityReason::Warmup)
    );
    assert_eq!(value(&actual[1]), Some(2.5));
    assert_eq!(
        reason(&actual[2]),
        Some(FeatureUnavailabilityReason::MissingDependency)
    );
}

#[test]
fn restart_replay_and_chunk_partitions_are_bit_identical_across_gaps_dependencies_and_calendar() {
    let calendar = TradingCalendarSnapshot::new(
        "a-share-equivalence",
        Venue::china_a_share("sse").unwrap(),
        0,
        2_000_000_000_000,
        vec![
            TradingSession {
                phase: SessionPhase::Continuous,
                start_local: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
                end_local: NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
            },
            TradingSession {
                phase: SessionPhase::Continuous,
                start_local: NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
                end_local: NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            },
        ],
        Vec::<DayEvidence>::new(),
    )
    .unwrap();
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![
            node(
                "mean",
                FeatureOperator::RollingMean,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("window".into(), json!(2))]),
            ),
            node(
                "day",
                FeatureOperator::TradingDayOfWeek,
                Vec::new(),
                BTreeMap::new(),
            ),
        ],
        outputs: vec![
            FeatureOutput {
                name: "mean".into(),
                node_id: "mean".into(),
            },
            FeatureOutput {
                name: "day".into(),
                node_id: "day".into(),
            },
        ],
    })
    .unwrap();
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        factors: vec![FeatureFactor {
            alias: "momentum".into(),
            parameters: Vec::new(),
            output_names: vec!["score".into()],
            warmup_bars: 0,
        }],
        slots: vec![FeatureSlot {
            name: "factor-score".into(),
            source: FeatureSource::External {
                dependency_alias: "momentum".into(),
                output: "score".into(),
            },
            warmup_bars: 0,
        }],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    // 2024-03-11 SSE morning session, minute cadence in venue-local time.
    let base_time = 1_710_120_600_000;
    let obs = |offset: i64, close: &str, dependency: Option<f64>| {
        let time = base_time + offset * 60_000;
        FeatureInputEvent::observation(
            FeatureEvaluationInput::new("600000", time, time, bar(time, close, "1", "1"))
                .with_calendar(calendar.clone())
                .with_dependency(FeatureDependencyInput::external(
                    "momentum", "score", dependency, time,
                )),
        )
    };
    let events = vec![
        obs(0, "10", Some(1.0)),
        obs(1, "12", None),
        FeatureInputEvent::bar_gap("600000", base_time + 2 * 60_000, base_time + 2 * 60_000),
        obs(3, "14", Some(2.0)),
        FeatureInputEvent::scheduled_closure("600000", base_time + 4 * 60_000),
        obs(5, "16", Some(3.0)),
    ];
    let engine = FeatureEngine::new(identity());
    let reference = engine.evaluate_batch(plan.clone(), &events).unwrap();
    assert!(
        reference
            .iter()
            .any(|observation| reason(observation) == Some(FeatureUnavailabilityReason::BarGap))
    );
    assert!(
        reference.iter().any(|observation| reason(observation)
            == Some(FeatureUnavailabilityReason::MissingDependency))
    );

    let mut stateful = engine.evaluator(plan.clone()).unwrap();
    let mut one_at_a_time = Vec::new();
    for event in &events {
        one_at_a_time.extend(stateful.observe(event.clone()).unwrap());
    }
    assert_eq!(one_at_a_time, reference);

    for chunk_size in [2usize, 3, 4, events.len()] {
        let mut chunked = engine.evaluator(plan.clone()).unwrap();
        let mut observations = Vec::new();
        for chunk in events.chunks(chunk_size) {
            observations.extend(chunked.evaluate_batch(chunk).unwrap());
        }
        assert_eq!(observations, reference);
    }

    let replayed = FeatureEngine::new(identity())
        .evaluate_batch(plan, &events)
        .unwrap();
    assert_eq!(replayed, reference);
}

#[test]
fn signal_dependency_slots_share_batch_and_stateful_evaluation() {
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        slots: vec![FeatureSlot {
            name: "forecast".into(),
            source: FeatureSource::Signal {
                dataset_id: "a".repeat(64),
                signal_name: "forecast".into(),
                snapshot_id: "snapshot".into(),
                instrument_id: "BTC-USD".into(),
                venue: "okx".into(),
                bar_interval: "1m".into(),
                contract: json!({
                    "name": "forecast",
                    "predictionKind": {"kind": "probability"},
                    "forecastTarget": {"kind": "builtin", "target": "future-close-up"},
                    "valueScale": {"kind": "probability"},
                    "horizonBars": 1
                }),
                producer_segments: vec![json!({"segment": 1})],
                artifact_provenance: json!({"sha256": "artifact"}),
                evidence_state: "unknown".into(),
                component_lock: vec![],
            },
            warmup_bars: 0,
        }],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    let signal = |time: i64, value: Option<f64>| {
        FeatureInputEvent::observation(
            FeatureEvaluationInput::new("BTC-USD", time, time, bar(time, "10", "1", "1"))
                .with_dependency(FeatureDependencyInput::signal(
                    "a".repeat(64),
                    "forecast",
                    value,
                    time,
                )),
        )
    };
    let events = vec![signal(1, Some(0.4)), signal(2, None), signal(3, Some(0.7))];
    let engine = FeatureEngine::new(identity());
    let reference = engine.evaluate_batch(plan.clone(), &events).unwrap();
    assert_eq!(value(&reference[0]), Some(0.4));
    assert_eq!(
        reason(&reference[1]),
        Some(FeatureUnavailabilityReason::MissingDependency)
    );
    assert_eq!(value(&reference[2]), Some(0.7));

    let mut stateful = engine.evaluator(plan.clone()).unwrap();
    let mut one_at_a_time = Vec::new();
    for event in &events {
        one_at_a_time.extend(stateful.observe(event.clone()).unwrap());
    }
    assert_eq!(one_at_a_time, reference);

    let replayed = FeatureEngine::new(identity())
        .evaluate_batch(plan, &events)
        .unwrap();
    assert_eq!(replayed, reference);
}

#[test]
fn cross_sectional_batches_are_equivalent_across_replay_and_partitioning() {
    let plan = cross_sectional_plan(
        vec![cross_sectional_node(
            "rank",
            FeatureOperator::CrossSectionalRank,
            BTreeMap::new(),
        )],
        &[("rank", "rank")],
    );
    let batches = vec![
        cross_sectional_batch(
            1,
            cross_sectional_universe(1, &["A", "B", "C"], UniverseEvidenceState::Observed),
            &[("A", "10"), ("B", "12"), ("C", "11")],
        ),
        cross_sectional_batch(
            2,
            cross_sectional_universe(2, &["A", "B"], UniverseEvidenceState::Observed),
            &[("A", "13"), ("B", "15")],
        ),
    ];
    let engine = FeatureEngine::new(identity());
    let reference = engine.evaluate_batch(plan.clone(), &batches).unwrap();
    let mut stateful = engine.evaluator(plan.clone()).unwrap();
    let mut one_at_a_time = Vec::new();
    for batch in &batches {
        one_at_a_time.extend(stateful.observe(batch.clone()).unwrap());
    }
    assert_eq!(one_at_a_time, reference);
    let replayed = FeatureEngine::new(identity())
        .evaluate_batch(plan, &batches)
        .unwrap();
    assert_eq!(replayed, reference);
}

fn observation_map(
    observations: &[FeatureObservation],
) -> HashMap<(String, String), FeatureObservation> {
    observations
        .iter()
        .cloned()
        .map(|observation| {
            (
                (
                    observation.instrument_id.clone(),
                    observation.output_name.clone(),
                ),
                observation,
            )
        })
        .collect()
}

#[test]
fn decimal_projection_and_backward_returns_are_causal() {
    let plan = plan(
        vec![
            node(
                "close",
                FeatureOperator::CheckedArithmetic,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::new(),
            ),
            node(
                "return",
                FeatureOperator::BackwardSimpleReturn,
                vec![FeatureInput::Node {
                    node_id: "close".into(),
                    definition_hash: None,
                }],
                BTreeMap::new(),
            ),
        ],
        &[("return", "return")],
    );
    let first_bar = bar(1, "100.000000", "0", "0");
    assert_eq!(first_bar.close.as_ref().unwrap().as_str(), "100.000000");
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .evaluate_batch(&[
            FeatureInputEvent::observation(FeatureEvaluationInput::new("BTC-USD", 1, 1, first_bar)),
            event(2, "110", "0", "0"),
            event(3, "105", "0", "0"),
        ])
        .unwrap();
    assert_eq!(
        reason(&observations[0]),
        Some(FeatureUnavailabilityReason::Warmup)
    );
    assert_close(value(&observations[1]), 0.1);
    assert_close(value(&observations[2]), 105.0 / 110.0 - 1.0);
}

#[test]
fn rolling_state_resets_on_gaps_but_not_scheduled_closures() {
    let plan = plan(
        vec![node(
            "mean",
            FeatureOperator::RollingMean,
            vec![FeatureInput::Market {
                field: MarketField::Close,
            }],
            BTreeMap::from([("window".into(), json!(2))]),
        )],
        &[("mean", "mean")],
    );
    let mut evaluator = FeatureEngine::new(identity())
        .evaluator(plan.clone())
        .unwrap();
    let observations = evaluator
        .evaluate_batch(&[
            event(1, "1", "0", "0"),
            event(2, "3", "0", "0"),
            FeatureInputEvent::bar_gap("BTC-USD", 3, 3),
            event(4, "5", "0", "0"),
        ])
        .unwrap();
    assert_eq!(
        reason(&observations[0]),
        Some(FeatureUnavailabilityReason::Warmup)
    );
    assert_eq!(value(&observations[1]), Some(2.0));
    assert_eq!(
        reason(&observations[2]),
        Some(FeatureUnavailabilityReason::BarGap)
    );
    assert_eq!(
        reason(&observations[3]),
        Some(FeatureUnavailabilityReason::Warmup)
    );

    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .evaluate_batch(&[
            event(1, "1", "0", "0"),
            event(2, "3", "0", "0"),
            FeatureInputEvent::scheduled_closure("BTC-USD", 3),
            event(4, "5", "0", "0"),
        ])
        .unwrap();
    assert_eq!(value(&observations[2]), Some(4.0));
}

#[test]
fn rolling_variants_and_realized_volatility_use_full_windows() {
    let plan = plan(
        vec![
            node(
                "mean",
                FeatureOperator::RollingMean,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("window".into(), json!(2))]),
            ),
            node(
                "std",
                FeatureOperator::RollingPopulationStandardDeviation,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("window".into(), json!(2))]),
            ),
            node(
                "min",
                FeatureOperator::RollingMinimum,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("window".into(), json!(2))]),
            ),
            node(
                "max",
                FeatureOperator::RollingMaximum,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("window".into(), json!(2))]),
            ),
            node(
                "quote-mean",
                FeatureOperator::RollingQuoteVolume,
                vec![FeatureInput::Market {
                    field: MarketField::QuoteVolume,
                }],
                BTreeMap::from([("window".into(), json!(2))]),
            ),
            node(
                "realized",
                FeatureOperator::RealizedVolatility,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("window".into(), json!(2))]),
            ),
        ],
        &[
            ("mean", "mean"),
            ("std", "std"),
            ("min", "min"),
            ("max", "max"),
            ("quote-mean", "quote-mean"),
            ("realized", "realized"),
        ],
    );
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .evaluate_batch(&[
            event(1, "100", "1", "10"),
            event(2, "110", "1", "20"),
            event(3, "100", "1", "30"),
        ])
        .unwrap();
    let last = &observations[12..18];
    assert_close(value(&last[0]), 105.0);
    assert_close(value(&last[1]), 5.0);
    assert_close(value(&last[2]), 100.0);
    assert_close(value(&last[3]), 110.0);
    assert_close(value(&last[4]), 25.0);
    let first_return = (110.0_f64 / 100.0).ln();
    let second_return = (100.0_f64 / 110.0).ln();
    let mean = (first_return + second_return) / 2.0;
    let expected = (((first_return - mean).powi(2) + (second_return - mean).powi(2)) / 2.0).sqrt();
    assert_close(value(&last[5]), expected);
}

#[test]
fn stateful_inputs_are_causal_and_invalid_parameters_are_rejected() {
    let mut pointwise = node(
        "return",
        FeatureOperator::BackwardSimpleReturn,
        vec![FeatureInput::Market {
            field: MarketField::Close,
        }],
        BTreeMap::new(),
    );
    pointwise.scope = FeatureScope::Pointwise;
    let error = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::Pointwise,
        nodes: vec![pointwise],
        outputs: vec![FeatureOutput {
            name: "return".into(),
            node_id: "return".into(),
        }],
    })
    .unwrap_err();
    assert!(error.codes().contains(&"invalid-operator-scope"));

    let invalid_window = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![node(
            "mean",
            FeatureOperator::RollingMean,
            vec![FeatureInput::Market {
                field: MarketField::Close,
            }],
            BTreeMap::from([("window".into(), json!(-1))]),
        )],
        outputs: vec![FeatureOutput {
            name: "mean".into(),
            node_id: "mean".into(),
        }],
    })
    .unwrap_err();
    assert!(invalid_window.codes().contains(&"invalid-rolling-window"));

    let mut evaluator = FeatureEngine::new(identity())
        .evaluator(plan(
            vec![node(
                "log-return",
                FeatureOperator::BackwardLogReturn,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::new(),
            )],
            &[("log-return", "log-return")],
        ))
        .unwrap();
    let error = evaluator
        .evaluate_batch(&[event(1, "0", "1", "1"), event(2, "1", "1", "1")])
        .unwrap();
    assert_eq!(
        reason(&error[1]),
        Some(FeatureUnavailabilityReason::UndefinedArithmetic)
    );
    let error = evaluator.observe(event(2, "2", "1", "1"));
    assert_eq!(error.unwrap_err().code(), "invalid-observation");
}

#[test]
fn pointwise_encoding_and_checked_division_are_typed() {
    let plan = plan(
        vec![
            node(
                "one-hot",
                FeatureOperator::OneHot,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("category".into(), json!(1))]),
            ),
            node(
                "sine",
                FeatureOperator::Sine,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("period".into(), json!(4))]),
            ),
            node(
                "cosine",
                FeatureOperator::Cosine,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::from([("period".into(), json!(4))]),
            ),
            node(
                "divide",
                FeatureOperator::CheckedArithmetic,
                vec![
                    FeatureInput::Market {
                        field: MarketField::Close,
                    },
                    FeatureInput::Market {
                        field: MarketField::QuoteVolume,
                    },
                ],
                BTreeMap::from([("operation".into(), json!("divide"))]),
            ),
        ],
        &[
            ("one-hot", "one-hot"),
            ("sine", "sine"),
            ("cosine", "cosine"),
            ("divide", "divide"),
        ],
    );
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .evaluate_batch(&[event(1, "1", "1", "0"), event(2, "2", "1", "4")])
        .unwrap();
    assert_eq!(value(&observations[0]), Some(1.0));
    assert_eq!(value(&observations[1]), Some(1.0));
    assert_close(value(&observations[2]), 0.0);
    assert_eq!(
        reason(&observations[3]),
        Some(FeatureUnavailabilityReason::UndefinedArithmetic)
    );
    assert_eq!(value(&observations[4]), Some(0.0));
    assert_close(value(&observations[5]), 0.0);
    assert_close(value(&observations[6]), -1.0);
    assert_close(value(&observations[7]), 0.5);
}

#[test]
fn volume_and_undefined_arithmetic_keep_typed_missingness() {
    let plan = plan(
        vec![
            node(
                "return",
                FeatureOperator::BackwardSimpleReturn,
                vec![FeatureInput::Market {
                    field: MarketField::Close,
                }],
                BTreeMap::new(),
            ),
            node(
                "quote",
                FeatureOperator::QuoteVolume,
                vec![FeatureInput::Market {
                    field: MarketField::QuoteVolume,
                }],
                BTreeMap::new(),
            ),
            node(
                "zero",
                FeatureOperator::ZeroVolume,
                vec![FeatureInput::Market {
                    field: MarketField::BaseVolume,
                }],
                BTreeMap::new(),
            ),
            node(
                "amihud",
                FeatureOperator::AmihudIlliquidity,
                vec![
                    FeatureInput::Node {
                        node_id: "return".into(),
                        definition_hash: None,
                    },
                    FeatureInput::Node {
                        node_id: "quote".into(),
                        definition_hash: None,
                    },
                ],
                BTreeMap::new(),
            ),
        ],
        &[
            ("return", "return"),
            ("quote", "quote"),
            ("zero", "zero"),
            ("amihud", "amihud"),
        ],
    );
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .evaluate_batch(&[
            event(1, "100", "0", "1000"),
            event(2, "110", "3", "2000"),
            event(3, "120", "0", "0"),
        ])
        .unwrap();
    let by_name = observations
        .chunks(4)
        .map(|row| {
            row.iter()
                .map(|observation| (observation.output_name.clone(), observation.clone()))
                .collect::<HashMap<_, _>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(value(&by_name[0]["zero"]), Some(1.0));
    assert_eq!(value(&by_name[1]["quote"]), Some(2000.0));
    assert_eq!(value(&by_name[1]["zero"]), Some(0.0));
    assert_close(value(&by_name[1]["amihud"]), 0.1 / 2000.0);
    assert_eq!(
        reason(&by_name[2]["amihud"]),
        Some(FeatureUnavailabilityReason::UndefinedArithmetic)
    );
}

#[test]
fn calendar_features_use_venue_local_time_and_exclude_breaks() {
    let session = |phase: SessionPhase, start: (u32, u32), end: (u32, u32)| TradingSession {
        phase,
        start_local: NaiveTime::from_hms_opt(start.0, start.1, 0).unwrap(),
        end_local: NaiveTime::from_hms_opt(end.0, end.1, 0).unwrap(),
    };
    let calendar = TradingCalendarSnapshot::new(
        "a-share-test",
        Venue::china_a_share("sse").unwrap(),
        0,
        2_000_000_000_000,
        vec![
            session(SessionPhase::Continuous, (9, 30), (11, 30)),
            session(SessionPhase::Break, (11, 30), (13, 0)),
            session(SessionPhase::Continuous, (13, 0), (15, 0)),
        ],
        Vec::<DayEvidence>::new(),
    )
    .unwrap();
    let nodes = [
        ("day", FeatureOperator::TradingDayOfWeek),
        ("month", FeatureOperator::TradingMonth),
        ("from-open", FeatureOperator::MinutesFromSessionOpen),
        ("to-close", FeatureOperator::MinutesToSessionClose),
        ("progress", FeatureOperator::SessionProgress),
    ]
    .into_iter()
    .map(|(id, operator)| node(id, operator, Vec::new(), BTreeMap::new()))
    .collect();
    let plan = plan(
        nodes,
        &[
            ("day", "day"),
            ("month", "month"),
            ("from-open", "from-open"),
            ("to-close", "to-close"),
            ("progress", "progress"),
        ],
    );
    let input =
        FeatureEvaluationInput::new("600000", 1_710_126_000_000, 1, bar(1, "10", "1", "10"))
            .with_calendar(calendar);
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .observe(FeatureInputEvent::observation(input))
        .unwrap();
    assert_eq!(value(&observations[0]), Some(0.0));
    assert_eq!(value(&observations[1]), Some(3.0));
    assert_eq!(value(&observations[2]), Some(90.0));
    assert_eq!(value(&observations[3]), Some(30.0));
    assert_eq!(value(&observations[4]), Some(0.375));
}

#[test]
fn calendar_closures_are_excluded_from_session_progress() {
    let venue = Venue::china_a_share("sse").unwrap();
    let timestamp = 1_710_126_000_000;
    let date = adaq_data_core::market::TradingDate::from_utc_ms(&venue, timestamp).unwrap();
    let local = |hour, minute| {
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
    let calendar = TradingCalendarSnapshot::new(
        "a-share-closure-test",
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
        vec![DayEvidence {
            date,
            day_kind: adaq_data_core::market::DayKind::TradingDay,
            session_override: None,
            closures: vec![adaq_data_core::market::ScheduledClosure {
                kind: adaq_data_core::market::ScheduledClosureKind::SpecialClosure,
                start_ms: local(10, 0),
                end_ms: local(10, 30),
                reason: Some("test closure".into()),
            }],
        }],
    )
    .unwrap();
    let plan = plan(
        vec![
            node(
                "from-open",
                FeatureOperator::MinutesFromSessionOpen,
                Vec::new(),
                BTreeMap::new(),
            ),
            node(
                "to-close",
                FeatureOperator::MinutesToSessionClose,
                Vec::new(),
                BTreeMap::new(),
            ),
            node(
                "progress",
                FeatureOperator::SessionProgress,
                Vec::new(),
                BTreeMap::new(),
            ),
        ],
        &[
            ("from-open", "from-open"),
            ("to-close", "to-close"),
            ("progress", "progress"),
        ],
    );
    let times = [local(9, 45), local(10, 15), local(10, 45)];
    let events = times
        .into_iter()
        .map(|time| {
            FeatureInputEvent::observation(
                FeatureEvaluationInput::new("sse:600000", time, time, bar(time, "10", "1", "1"))
                    .with_calendar(calendar.clone()),
            )
        })
        .collect::<Vec<_>>();
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator.evaluate_batch(&events).unwrap();
    assert_close(value(&observations[0]), 15.0);
    assert_close(value(&observations[1]), 75.0);
    assert_close(value(&observations[2]), 15.0 / 210.0);
    assert_eq!(
        reason(&observations[3]),
        Some(FeatureUnavailabilityReason::InsufficientCoverage)
    );
    assert_eq!(
        reason(&observations[4]),
        Some(FeatureUnavailabilityReason::InsufficientCoverage)
    );
    assert_eq!(
        reason(&observations[5]),
        Some(FeatureUnavailabilityReason::InsufficientCoverage)
    );
    assert_close(value(&observations[6]), 45.0);
    assert_close(value(&observations[7]), 45.0);
    assert_close(value(&observations[8]), 45.0 / 210.0);
}

#[test]
fn split_and_dividend_features_are_forward_and_causally_available() {
    let nodes = vec![
        node(
            "split",
            FeatureOperator::CausalSplitAdjustment,
            vec![FeatureInput::Market {
                field: MarketField::Close,
            }],
            BTreeMap::new(),
        ),
        node(
            "dividend",
            FeatureOperator::DividendTotalReturn,
            vec![FeatureInput::Market {
                field: MarketField::Close,
            }],
            BTreeMap::new(),
        ),
    ];
    let plan = plan(nodes, &[("split", "split"), ("dividend", "dividend")]);
    let actions = vec![
        CorporateAction::split("600000", 2, 2, "1").unwrap(),
        CorporateAction::dividend(
            "600000",
            2,
            2,
            "10",
            Some(adaq_feature_engine::CanonicalDecimal::new("100").unwrap()),
        )
        .unwrap(),
    ];
    let second = FeatureEvaluationInput::new("600000", 2, 2, bar(2, "110", "1", "1"))
        .with_corporate_actions(actions);
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .evaluate_batch(&[
            event(1, "100", "1", "1"),
            FeatureInputEvent::observation(second),
            FeatureInputEvent::observation(FeatureEvaluationInput::new(
                "600000",
                3,
                3,
                bar(3, "120", "1", "1"),
            )),
        ])
        .unwrap();
    assert_eq!(value(&observations[0]), Some(100.0));
    assert_eq!(value(&observations[1]), Some(100.0));
    assert_eq!(value(&observations[2]), Some(55.0));
    assert_close(value(&observations[3]), 121.0);
    assert_eq!(value(&observations[4]), Some(60.0));
    assert_close(value(&observations[5]), 132.0);
    assert_eq!(
        match observations[2].value {
            FeatureObservationValue::Available {
                available_at_ms, ..
            } => available_at_ms,
            _ => 0,
        },
        2
    );
}

#[test]
fn ashare_corporate_actions_retain_instrument_and_evidence_identity() {
    let instrument = InstrumentId::new(Venue::china_a_share("sse").unwrap(), "600000").unwrap();
    let action = adaq_data_core::a_share::AshareCorporateAction {
        instrument,
        provider_symbol: "sh.600000".into(),
        kind: adaq_data_core::a_share::AshareCorporateActionKind::CashAndShareDistribution,
        effective_at_ms: Some(2),
        announced_at_ms: Some(1),
        available_at_ms: 2,
        cash_per_share: Some("1".into()),
        shares_per_share: Some("1".into()),
        raw_payload: json!({"event": "split-and-dividend"}),
    };
    let actions = FeatureMarketBar::from_ashare_action(&action).unwrap();
    assert_eq!(actions.len(), 2);
    for action in actions {
        let encoded = serde_json::to_value(action).unwrap();
        assert_eq!(encoded["instrumentId"], "sse:600000");
        assert!(!encoded["evidenceId"].as_str().unwrap().is_empty());
    }
}

#[test]
fn batch_and_stateful_observation_paths_are_identical() {
    let plan = plan(
        vec![node(
            "mean",
            FeatureOperator::RollingMean,
            vec![FeatureInput::Market {
                field: MarketField::Close,
            }],
            BTreeMap::from([("window".into(), json!(3))]),
        )],
        &[("mean", "mean")],
    );
    let events = (1..=6)
        .map(|time| event(time, &(time * 2).to_string(), "1", "2"))
        .collect::<Vec<_>>();
    let engine = FeatureEngine::new(identity());
    let batch = engine.evaluate_batch(plan.clone(), &events).unwrap();
    let mut stateful = engine.evaluator(plan).unwrap();
    let mut observed = Vec::new();
    for chunk in events.chunks(2) {
        for event in chunk {
            observed.extend(stateful.observe(event.clone()).unwrap());
        }
    }
    assert_eq!(batch, observed);
}

#[test]
fn indicator_nodes_use_the_pinned_indicator_engine_and_validate_output() {
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![node(
            "rsi",
            FeatureOperator::Indicator { id: "rsi".into() },
            vec![FeatureInput::Market {
                field: MarketField::Close,
            }],
            BTreeMap::from([
                ("output".into(), json!("value")),
                ("time-period".into(), json!(2)),
            ]),
        )],
        outputs: vec![FeatureOutput {
            name: "rsi".into(),
            node_id: "rsi".into(),
        }],
    })
    .unwrap();
    let native_engine = FeatureEngine::native().unwrap();
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        engine_identity: native_engine.identity().clone(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    let events = (1..=8)
        .map(|time| event(time, &(100 + time).to_string(), "1", "1"))
        .collect::<Vec<_>>();
    let observations = native_engine.evaluate_batch(plan.clone(), &events).unwrap();
    assert!(reason(observations.last().unwrap()).is_none());
    assert!(value(observations.last().unwrap()).unwrap().is_finite());

    let partial_events = (1..=8)
        .map(|time| {
            FeatureInputEvent::observation(FeatureEvaluationInput::new(
                "BTC-USD",
                time,
                time,
                FeatureMarketBar {
                    open_time_ms: time,
                    open: None,
                    high: None,
                    low: None,
                    close: Some(
                        adaq_feature_engine::CanonicalDecimal::new(&(100 + time).to_string())
                            .unwrap(),
                    ),
                    base_volume: None,
                    quote_volume: None,
                },
            ))
        })
        .collect::<Vec<_>>();
    let observations = native_engine.evaluate_batch(plan, &partial_events).unwrap();
    assert!(reason(observations.last().unwrap()).is_none());

    let adx_definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![node(
            "adx",
            FeatureOperator::Indicator { id: "adx".into() },
            vec![
                FeatureInput::Market {
                    field: MarketField::High,
                },
                FeatureInput::Market {
                    field: MarketField::Low,
                },
                FeatureInput::Market {
                    field: MarketField::Close,
                },
            ],
            BTreeMap::from([
                ("output".into(), json!("value")),
                ("time-period".into(), json!(2)),
            ]),
        )],
        outputs: vec![FeatureOutput {
            name: "adx".into(),
            node_id: "adx".into(),
        }],
    })
    .unwrap();
    let adx_plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![adx_definition],
        engine_identity: native_engine.identity().clone(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    let adx_events = (1..=12)
        .map(|time| {
            let close = 100 + time;
            FeatureInputEvent::observation(FeatureEvaluationInput::new(
                "BTC-USD",
                time,
                time,
                FeatureMarketBar::complete(
                    time,
                    (close - 1).to_string(),
                    (close + 2).to_string(),
                    (close - 2).to_string(),
                    close.to_string(),
                    "1",
                    "1",
                )
                .unwrap(),
            ))
        })
        .collect::<Vec<_>>();
    let observations = native_engine.evaluate_batch(adx_plan, &adx_events).unwrap();
    assert!(reason(observations.last().unwrap()).is_none());
    assert!(value(observations.last().unwrap()).unwrap().is_finite());
}

#[test]
fn future_return_direction_is_rejected_at_definition_freeze() {
    let error = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![node(
            "return",
            FeatureOperator::BackwardSimpleReturn,
            vec![FeatureInput::Market {
                field: MarketField::Close,
            }],
            BTreeMap::from([("direction".into(), json!("forward"))]),
        )],
        outputs: vec![FeatureOutput {
            name: "return".into(),
            node_id: "return".into(),
        }],
    })
    .unwrap_err();
    assert!(error.codes().contains(&"future-return-not-allowed"));
}

#[test]
fn cross_sectional_rank_percentile_and_zscore_are_deterministic() {
    let plan = cross_sectional_plan(
        vec![
            cross_sectional_node("rank", FeatureOperator::CrossSectionalRank, BTreeMap::new()),
            cross_sectional_node(
                "reverse-rank",
                FeatureOperator::CrossSectionalRank,
                BTreeMap::from([("reverse".into(), json!(true))]),
            ),
            cross_sectional_node(
                "percentile",
                FeatureOperator::CrossSectionalPercentile,
                BTreeMap::new(),
            ),
            cross_sectional_node(
                "z-score",
                FeatureOperator::CrossSectionalZScore,
                BTreeMap::new(),
            ),
        ],
        &[
            ("rank", "rank"),
            ("reverse-rank", "reverse-rank"),
            ("percentile", "percentile"),
            ("z-score", "z-score"),
        ],
    );
    let universe = cross_sectional_universe(10, &["A", "B", "C"], UniverseEvidenceState::Observed);
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .observe(cross_sectional_batch(
            10,
            universe,
            &[("C", "20"), ("A", "10"), ("B", "20")],
        ))
        .unwrap();
    let observations = observation_map(&observations);

    assert_eq!(
        value(&observations[&(String::from("A"), String::from("rank"))]),
        Some(1.0)
    );
    assert_eq!(
        value(&observations[&(String::from("B"), String::from("rank"))]),
        Some(2.5)
    );
    assert_eq!(
        value(&observations[&(String::from("C"), String::from("reverse-rank"))]),
        Some(1.5)
    );
    assert_eq!(
        value(&observations[&(String::from("A"), String::from("percentile"))]),
        Some(0.0)
    );
    assert_eq!(
        value(&observations[&(String::from("B"), String::from("percentile"))]),
        Some(0.75)
    );
    assert_close(
        value(&observations[&(String::from("A"), String::from("z-score"))]),
        -std::f64::consts::SQRT_2,
    );
    assert_eq!(
        observations[&(String::from("B"), String::from("rank"))]
            .cross_sectional_coverage
            .as_ref()
            .unwrap()
            .available_count,
        3
    );
}

#[test]
fn cross_sectional_nodes_can_consume_lower_scope_features() {
    let mut close = node(
        "close",
        FeatureOperator::CheckedArithmetic,
        vec![FeatureInput::Market {
            field: MarketField::Close,
        }],
        BTreeMap::new(),
    );
    close.scope = FeatureScope::Pointwise;
    let rank = FeatureNode {
        id: "rank".into(),
        operator: FeatureOperator::CrossSectionalRank,
        scope: FeatureScope::CrossSectional,
        inputs: vec![FeatureInput::Node {
            node_id: "close".into(),
            definition_hash: None,
        }],
        parameters: BTreeMap::new(),
        warmup_bars: 0,
    };
    let plan = cross_sectional_plan(vec![close, rank], &[("rank", "rank")]);
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .observe(cross_sectional_batch(
            10,
            cross_sectional_universe(10, &["A", "B"], UniverseEvidenceState::Observed),
            &[("B", "20"), ("A", "10")],
        ))
        .unwrap();
    let observations = observation_map(&observations);
    assert_eq!(
        value(&observations[&(String::from("A"), String::from("rank"))]),
        Some(1.0)
    );
    assert_eq!(
        value(&observations[&(String::from("B"), String::from("rank"))]),
        Some(2.0)
    );
}

#[test]
fn cross_sectional_coverage_preserves_missing_members_and_actual_coverage() {
    let plan = cross_sectional_plan(
        vec![cross_sectional_node(
            "rank",
            FeatureOperator::CrossSectionalRank,
            BTreeMap::from([
                ("minimum-count".into(), json!(2)),
                ("minimum-coverage".into(), json!(0.5)),
            ]),
        )],
        &[("rank", "rank")],
    );
    let universe = cross_sectional_universe(
        10,
        &["A", "B", "C", "D"],
        UniverseEvidenceState::Reconstructed,
    );
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .observe(cross_sectional_batch(
            10,
            universe,
            &[("C", "30"), ("A", "10")],
        ))
        .unwrap();
    let observations = observation_map(&observations);

    assert_eq!(
        value(&observations[&(String::from("A"), String::from("rank"))]),
        Some(1.0)
    );
    assert_eq!(
        value(&observations[&(String::from("C"), String::from("rank"))]),
        Some(2.0)
    );
    assert_eq!(
        reason(&observations[&(String::from("B"), String::from("rank"))]),
        Some(FeatureUnavailabilityReason::MissingMarketInput)
    );
    assert_eq!(
        reason(&observations[&(String::from("D"), String::from("rank"))]),
        Some(FeatureUnavailabilityReason::MissingMarketInput)
    );
    let coverage = observations[&(String::from("A"), String::from("rank"))]
        .cross_sectional_coverage
        .as_ref()
        .unwrap();
    assert_eq!(coverage.universe_count, 4);
    assert_eq!(coverage.available_count, 2);
    assert_eq!(coverage.actual_coverage, 0.5);
    assert_eq!(
        coverage.evidence_state,
        UniverseEvidenceState::Reconstructed
    );
}

#[test]
fn cross_sectional_default_full_coverage_and_singleton_formulas_are_unavailable() {
    let plan = cross_sectional_plan(
        vec![
            cross_sectional_node("rank", FeatureOperator::CrossSectionalRank, BTreeMap::new()),
            cross_sectional_node(
                "percentile",
                FeatureOperator::CrossSectionalPercentile,
                BTreeMap::new(),
            ),
            cross_sectional_node(
                "z-score",
                FeatureOperator::CrossSectionalZScore,
                BTreeMap::new(),
            ),
        ],
        &[
            ("rank", "rank"),
            ("percentile", "percentile"),
            ("z-score", "z-score"),
        ],
    );
    let mut evaluator = FeatureEngine::new(identity())
        .evaluator(plan.clone())
        .unwrap();
    let observations = evaluator
        .observe(cross_sectional_batch(
            10,
            cross_sectional_universe(10, &["A", "B"], UniverseEvidenceState::Observed),
            &[("A", "10")],
        ))
        .unwrap();
    let observations = observation_map(&observations);
    assert_eq!(
        reason(&observations[&(String::from("A"), String::from("rank"))]),
        Some(FeatureUnavailabilityReason::InsufficientCoverage)
    );
    assert_eq!(
        reason(&observations[&(String::from("B"), String::from("rank"))]),
        Some(FeatureUnavailabilityReason::MissingMarketInput)
    );

    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let observations = evaluator
        .observe(cross_sectional_batch(
            20,
            cross_sectional_universe(20, &["A"], UniverseEvidenceState::Observed),
            &[("A", "10")],
        ))
        .unwrap();
    let observations = observation_map(&observations);
    assert_eq!(
        value(&observations[&(String::from("A"), String::from("rank"))]),
        Some(1.0)
    );
    assert_eq!(
        reason(&observations[&(String::from("A"), String::from("percentile"))]),
        Some(FeatureUnavailabilityReason::UndefinedArithmetic)
    );
    assert_eq!(
        reason(&observations[&(String::from("A"), String::from("z-score"))]),
        Some(FeatureUnavailabilityReason::UndefinedArithmetic)
    );
}

#[test]
fn cross_sectional_unknown_universe_is_complete_and_mixed_markets_are_rejected() {
    let plan = cross_sectional_plan(
        vec![cross_sectional_node(
            "rank",
            FeatureOperator::CrossSectionalRank,
            BTreeMap::new(),
        )],
        &[("rank", "rank")],
    );
    let universe = cross_sectional_universe(10, &["A", "B"], UniverseEvidenceState::Unknown);
    let mut evaluator = FeatureEngine::new(identity())
        .evaluator(plan.clone())
        .unwrap();
    let observations = evaluator
        .observe(cross_sectional_batch(10, universe, &[("A", "10")]))
        .unwrap();
    assert_eq!(observations.len(), 2);
    assert!(observations.iter().all(|observation| {
        reason(observation) == Some(FeatureUnavailabilityReason::UnknownUniverse)
    }));

    let universe = cross_sectional_universe(20, &["A", "B"], UniverseEvidenceState::Observed);
    let us_context = cross_sectional_context();
    let crypto_context = FeatureMarketContext::new(
        Venue::crypto_spot("okx").unwrap(),
        VenueKind::CryptoSpot,
        BarInterval::OneDay,
        PriceBasis::Unadjusted,
        "USD",
    )
    .unwrap();
    let batch = FeatureInputEvent::cross_sectional_batch(
        20,
        universe,
        vec![
            FeatureEvaluationInput::new("A", 20, 20, bar(20, "10", "1", "1"))
                .with_market_context(us_context),
            FeatureEvaluationInput::new("B", 20, 20, bar(20, "20", "1", "1"))
                .with_market_context(crypto_context),
        ],
    );
    let mut evaluator = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let error = evaluator.observe(batch).unwrap_err();
    assert_eq!(error.code(), "invalid-observation");
    assert_eq!(error.diagnostic, "cross-sectional-market-context-mismatch");
}

#[test]
fn cross_sectional_preview_keeps_every_member_and_input_order_does_not_change_output() {
    let plan = cross_sectional_plan(
        vec![cross_sectional_node(
            "rank",
            FeatureOperator::CrossSectionalRank,
            BTreeMap::new(),
        )],
        &[("rank", "rank")],
    );
    let universe = cross_sectional_universe(10, &["A", "B", "C"], UniverseEvidenceState::Observed);
    let first = cross_sectional_batch(10, universe.clone(), &[("C", "30"), ("A", "10")]);
    let second = cross_sectional_batch(
        20,
        cross_sectional_universe(20, &["A", "B", "C"], UniverseEvidenceState::Observed),
        &[("A", "11"), ("B", "21"), ("C", "31")],
    );
    let mut evaluator = FeatureEngine::new(identity())
        .evaluator(plan.clone())
        .unwrap();
    let preview = evaluator.evaluate_batch(&[first, second]).unwrap();
    assert_eq!(preview.len(), 6);
    assert_eq!(
        preview
            .iter()
            .filter(|observation| observation.observation_time_ms == 10)
            .count(),
        3
    );

    let mut ordered = FeatureEngine::new(identity())
        .evaluator(plan.clone())
        .unwrap();
    let ordered = ordered
        .observe(cross_sectional_batch(
            10,
            universe,
            &[("A", "10"), ("C", "30")],
        ))
        .unwrap();
    let mut reversed = FeatureEngine::new(identity()).evaluator(plan).unwrap();
    let reversed = reversed
        .observe(cross_sectional_batch(
            10,
            cross_sectional_universe(10, &["A", "B", "C"], UniverseEvidenceState::Observed),
            &[("C", "30"), ("A", "10")],
        ))
        .unwrap();
    assert_eq!(observation_map(&ordered), observation_map(&reversed));
}
