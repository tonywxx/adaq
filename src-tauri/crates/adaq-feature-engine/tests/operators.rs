use std::collections::{BTreeMap, HashMap};

use adaq_data_core::market::{
    DayEvidence, SessionPhase, TradingCalendarSnapshot, TradingSession, Venue,
};
use adaq_feature_engine::{
    CorporateAction, DefinitionDraft, FeatureDefinition, FeatureEngine, FeatureEngineIdentity,
    FeatureEvaluationInput, FeatureInput, FeatureInputEvent, FeatureMarketBar, FeatureNode,
    FeatureObservation, FeatureObservationValue, FeatureOperator, FeatureOutput, FeaturePlan,
    FeaturePlanDraft, FeatureScope, FeatureUnavailabilityReason, MarketField,
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
                    },
                    FeatureInput::Node {
                        node_id: "quote".into(),
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
        CorporateAction::split(2, 2, "2").unwrap(),
        CorporateAction::dividend(
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
    assert_eq!(value(&observations[2]), Some(220.0));
    assert_close(value(&observations[3]), 121.0);
    assert_eq!(value(&observations[4]), Some(240.0));
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
    let observations = native_engine.evaluate_batch(plan, &events).unwrap();
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
