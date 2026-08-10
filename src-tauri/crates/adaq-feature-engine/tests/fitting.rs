use std::collections::BTreeMap;

use adaq_feature_engine::{
    DefinitionDraft, FeatureDefinition, FeatureEngine, FeatureEngineIdentity,
    FeatureEvaluationInput, FeatureInput, FeatureInputEvent, FeatureMarketBar, FeatureNode,
    FeatureObservation, FeatureObservationValue, FeatureOperator, FeatureOutput, FeaturePlan,
    FeaturePlanDraft, FeatureReference, FeatureScope, FeatureUnavailabilityReason,
    FittedArtifactBinding, FittedParameters, FittedTransformationArtifact,
    FittedTransformationValue, FittingAlgorithm, FittingApplyError, FittingScope, MarketField,
    NEAREST_RANK_QUANTILE_VERSION, ObservationRange, TransformationFittingAttemptStatus,
    TransformationFittingProtocol, TransformationFittingProtocolDraft, TransformationFittingStore,
    WinsorizationParameters,
};
use uuid::Uuid;

fn identity() -> FeatureEngineIdentity {
    FeatureEngineIdentity::for_tests()
}

fn reference(output_name: &str) -> FeatureReference {
    FeatureReference {
        definition_hash: "a".repeat(64),
        node_id: output_name.into(),
        output_name: output_name.into(),
    }
}

fn protocol(scope: FittingScope, algorithm: FittingAlgorithm) -> TransformationFittingProtocol {
    TransformationFittingProtocol::freeze(TransformationFittingProtocolDraft {
        input_feature: reference("close"),
        fitted_node_id: "standardized".into(),
        fitted_output: reference("standardized"),
        snapshot_id: "snapshot-1".into(),
        point_in_time_universe_id: "universe-1".into(),
        fitting_scope: scope,
        fitting_window: ObservationRange {
            start_time_ms: 0,
            end_time_ms: 100,
        },
        algorithm,
        minimum_samples: 2,
        engine_identity: identity(),
    })
    .unwrap()
}

fn sample(instrument_id: &str, time: i64, value: f64, available_at_ms: i64) -> FeatureObservation {
    FeatureObservation::available("close", instrument_id, time, value, available_at_ms)
        .unwrap()
        .with_feature_reference(reference("close"))
}

fn standardization_protocol(scope: FittingScope) -> TransformationFittingProtocol {
    protocol(scope, FittingAlgorithm::Standardization)
}

#[test]
fn fitting_requires_exact_feature_provenance() {
    let protocol = standardization_protocol(FittingScope::PooledUniverse);
    let mut mismatched = sample("btc", 10, 1.0, 10);
    mismatched.feature_reference = Some(reference("other"));
    let error = protocol
        .fit(&[mismatched, sample("btc", 20, 2.0, 20)], 1000)
        .unwrap_err();
    assert_eq!(error.code(), "fitting-input-feature-mismatch");
}

#[test]
fn protocol_identity_is_canonical_and_winsorization_version_is_explicit() {
    let protocol = protocol(
        FittingScope::PooledUniverse,
        FittingAlgorithm::Winsorization {
            lower_quantile: 0.25,
            upper_quantile: 0.75,
            quantile_method_version: NEAREST_RANK_QUANTILE_VERSION.into(),
        },
    );
    let bytes = protocol.to_json();
    assert_eq!(
        TransformationFittingProtocol::load_for_engine(&bytes, &identity()).unwrap(),
        protocol
    );
    assert_eq!(protocol.protocol_hash().len(), 64);
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("nearest-rank@1.0.0")
    );
}

#[test]
fn standardization_uses_population_variance_and_excludes_future_available_samples() {
    let protocol = standardization_protocol(FittingScope::PooledUniverse);
    let artifact = protocol
        .fit(
            &[
                sample("btc", 10, 10.0, 10),
                sample("btc", 20, 20.0, 20),
                sample("btc", 30, 30.0, 30),
                sample("btc", 40, 100.0, 200),
            ],
            999,
        )
        .unwrap();
    assert_eq!(artifact.eligible_at_ms(), 30);
    assert_eq!(artifact.created_at_ms(), 999);
    assert_eq!(artifact.protocol_hash(), protocol.protocol_hash());
    assert_eq!(
        FittedTransformationArtifact::load_for_engine(&artifact.to_json(), &identity()).unwrap(),
        artifact
    );
    match artifact.parameters_for("btc").unwrap() {
        FittedParameters::Standardization {
            mean,
            population_standard_deviation,
            sample_count,
        } => {
            assert_eq!(*mean, 20.0);
            assert!((*population_standard_deviation - (200.0_f64 / 3.0).sqrt()).abs() < 1e-12);
            assert_eq!(*sample_count, 3);
        }
        other => panic!("unexpected parameters: {other:?}"),
    }
    match artifact.apply_value("btc", 100, 30.0, 100).unwrap() {
        FittedTransformationValue::Available {
            value,
            available_at_ms,
        } => {
            assert!((value - 10.0 / (200.0_f64 / 3.0).sqrt()).abs() < 1e-12);
            assert_eq!(available_at_ms, 100);
        }
        other => panic!("unexpected value: {other:?}"),
    }
}

#[test]
fn winsorization_uses_nearest_rank_and_constant_standardization_is_unavailable() {
    let winsor = protocol(
        FittingScope::PooledUniverse,
        FittingAlgorithm::Winsorization {
            lower_quantile: 0.25,
            upper_quantile: 0.75,
            quantile_method_version: NEAREST_RANK_QUANTILE_VERSION.into(),
        },
    )
    .fit(
        &[
            sample("btc", 10, 1.0, 10),
            sample("btc", 20, 2.0, 20),
            sample("btc", 30, 3.0, 30),
            sample("btc", 40, 4.0, 40),
        ],
        1000,
    )
    .unwrap();
    match winsor.parameters_for("btc").unwrap() {
        FittedParameters::Winsorization(WinsorizationParameters {
            lower_value,
            upper_value,
            ..
        }) => {
            assert_eq!((*lower_value, *upper_value), (1.0, 3.0));
        }
        other => panic!("unexpected parameters: {other:?}"),
    }
    assert!(matches!(
        winsor.apply_value("btc", 100, 4.0, 100).unwrap(),
        FittedTransformationValue::Available { value, .. } if value == 3.0
    ));

    let constant = standardization_protocol(FittingScope::PooledUniverse)
        .fit(
            &[sample("btc", 10, 5.0, 10), sample("btc", 20, 5.0, 20)],
            1000,
        )
        .unwrap();
    assert!(matches!(
        constant.apply_value("btc", 100, 5.0, 100).unwrap(),
        FittedTransformationValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
    ));
}

#[test]
fn insufficient_samples_fail_without_publishing_an_artifact() {
    let protocol = TransformationFittingProtocol::freeze(TransformationFittingProtocolDraft {
        minimum_samples: 3,
        ..standardization_protocol(FittingScope::PooledUniverse).draft()
    })
    .unwrap();
    let error = protocol
        .fit(
            &[sample("btc", 10, 1.0, 10), sample("btc", 20, 2.0, 20)],
            1000,
        )
        .unwrap_err();
    assert_eq!(error.code(), "insufficient-samples");

    let mut store = TransformationFittingStore::new();
    let attempt = store.start("alice", &protocol).unwrap();
    store.mark_running("alice", &attempt.attempt_id).unwrap();
    store
        .fail("alice", &attempt.attempt_id, "insufficient-samples")
        .unwrap();
    assert!(store.artifacts_for_user("alice").unwrap().is_empty());
    assert_eq!(
        store
            .get_attempt("alice", &attempt.attempt_id)
            .unwrap()
            .status,
        TransformationFittingAttemptStatus::Failed
    );
}

#[test]
fn per_instrument_parameters_are_exact_and_walk_forward_rejects_future_artifacts() {
    let artifact = standardization_protocol(FittingScope::PerInstrument)
        .fit(
            &[
                sample("btc", 10, 10.0, 10),
                sample("btc", 20, 20.0, 20),
                sample("eth", 10, 100.0, 10),
                sample("eth", 20, 120.0, 20),
            ],
            1000,
        )
        .unwrap();
    assert!(artifact.parameters_for("btc").is_some());
    assert!(artifact.parameters_for("unknown").is_none());
    assert!(matches!(
        artifact.apply_value("unknown", 100, 10.0, 100).unwrap(),
        FittedTransformationValue::Unavailable(
            FeatureUnavailabilityReason::ArtifactMissingInstrument
        )
    ));

    let future = artifact.apply_value("btc", 99, 15.0, 99);
    assert!(matches!(
        future,
        Err(FittingApplyError::ArtifactNotAvailableForObservation { .. })
    ));
    let mut tampered = artifact.clone();
    tampered.eligible_at_ms += 1;
    assert!(matches!(
        tampered.apply_value("btc", 100, 15.0, 100),
        Err(FittingApplyError::InvalidArtifact)
    ));
}

#[test]
fn lifecycle_coalesces_reuses_retries_and_keeps_artifacts_user_scoped_and_locked() {
    let protocol = standardization_protocol(FittingScope::PooledUniverse);
    let artifact = protocol
        .fit(
            &[sample("btc", 10, 1.0, 10), sample("btc", 20, 2.0, 20)],
            1000,
        )
        .unwrap();
    let mut store = TransformationFittingStore::new();
    let first = store.start("alice", &protocol).unwrap();
    assert_eq!(
        store.start("alice", &protocol).unwrap().attempt_id,
        first.attempt_id
    );
    store.mark_running("alice", &first.attempt_id).unwrap();
    assert_eq!(
        store.start("alice", &protocol).unwrap().attempt_id,
        first.attempt_id
    );
    store
        .publish_completed("alice", &first.attempt_id, artifact.clone())
        .unwrap();
    assert_eq!(
        store.start("alice", &protocol).unwrap().attempt_id,
        first.attempt_id
    );

    let failed = store
        .start(
            "alice",
            &TransformationFittingProtocol::freeze(TransformationFittingProtocolDraft {
                snapshot_id: "other-snapshot".into(),
                ..protocol.draft()
            })
            .unwrap(),
        )
        .unwrap();
    store.mark_running("alice", &failed.attempt_id).unwrap();
    store
        .fail("alice", &failed.attempt_id, "fit-failed")
        .unwrap();
    let retry = store.retry("alice", &failed.attempt_id).unwrap();
    assert_ne!(retry.attempt_id, failed.attempt_id);
    assert_eq!(
        store.retry("alice", &failed.attempt_id).unwrap().attempt_id,
        retry.attempt_id
    );
    assert_eq!(
        store
            .get_attempt("alice", &failed.attempt_id)
            .unwrap()
            .status,
        TransformationFittingAttemptStatus::Failed
    );

    let second_attempt = store.start("bob", &protocol).unwrap();
    store
        .mark_running("bob", &second_attempt.attempt_id)
        .unwrap();
    let mut deduplicated_artifact = artifact.clone();
    deduplicated_artifact.created_at_ms += 1;
    store
        .publish_completed("bob", &second_attempt.attempt_id, deduplicated_artifact)
        .unwrap();
    assert_eq!(store.stored_artifact_count(), 1);
    assert!(
        store
            .artifact_for_user("bob", artifact.artifact_id())
            .is_ok()
    );
    assert!(
        store
            .artifact_for_user("alice", artifact.artifact_id())
            .is_ok()
    );
    assert!(
        store
            .artifact_for_user("mallory", artifact.artifact_id())
            .is_err()
    );

    store
        .reference_artifact("alice", artifact.artifact_id(), "plan-1")
        .unwrap();
    assert_eq!(
        store
            .delete_artifact("alice", artifact.artifact_id())
            .unwrap_err()
            .code(),
        "artifact-referenced"
    );
    store
        .unreference_artifact("alice", artifact.artifact_id(), "plan-1")
        .unwrap();
    store
        .delete_artifact("alice", artifact.artifact_id())
        .unwrap();
    assert!(
        store
            .artifact_for_user("alice", artifact.artifact_id())
            .is_err()
    );
    assert!(
        store
            .artifact_for_user("bob", artifact.artifact_id())
            .is_ok()
    );
}

#[test]
fn feature_evaluator_applies_bound_artifact_without_fitting_or_mutating_it() {
    let close = FeatureNode {
        id: "close".into(),
        operator: FeatureOperator::CheckedArithmetic,
        scope: FeatureScope::TimeSeries,
        inputs: vec![FeatureInput::Market {
            field: "close".into(),
        }],
        parameters: BTreeMap::new(),
        warmup_bars: 0,
    };
    let input_definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(10),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![close.clone()],
        outputs: vec![FeatureOutput {
            name: "close".into(),
            node_id: "close".into(),
        }],
    })
    .unwrap();
    let input_feature = FeatureReference {
        definition_hash: input_definition.definition_hash().into(),
        node_id: "close".into(),
        output_name: "close".into(),
    };
    let mut local_close = close.clone();
    local_close.inputs = vec![FeatureInput::Market {
        field: MarketField::High,
    }];
    let standardized = FeatureNode {
        id: "standardized".into(),
        operator: FeatureOperator::Standardization,
        scope: FeatureScope::TimeSeries,
        inputs: vec![FeatureInput::Node {
            node_id: "close".into(),
            definition_hash: Some(input_feature.definition_hash.clone()),
        }],
        parameters: BTreeMap::new(),
        warmup_bars: 0,
    };
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::new_v4(),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![local_close.clone(), standardized.clone()],
        outputs: vec![FeatureOutput {
            name: "standardized".into(),
            node_id: "standardized".into(),
        }],
    })
    .unwrap();
    let fitted_output = FeatureReference {
        definition_hash: definition.definition_hash().into(),
        node_id: "standardized".into(),
        output_name: "standardized".into(),
    };
    let protocol = TransformationFittingProtocol::freeze(TransformationFittingProtocolDraft {
        input_feature: input_feature.clone(),
        fitted_node_id: "standardized".into(),
        fitted_output: fitted_output.clone(),
        snapshot_id: "snapshot-1".into(),
        point_in_time_universe_id: "universe-1".into(),
        fitting_scope: FittingScope::PooledUniverse,
        fitting_window: ObservationRange {
            start_time_ms: 0,
            end_time_ms: 100,
        },
        algorithm: FittingAlgorithm::Standardization,
        minimum_samples: 2,
        engine_identity: identity(),
    })
    .unwrap();
    let fitting_sample = |time: i64, value: f64| {
        FeatureObservation::available("close", "btc", time, value, time)
            .unwrap()
            .with_feature_reference(input_feature.clone())
    };
    let artifact = protocol
        .fit(&[fitting_sample(10, 10.0), fitting_sample(20, 20.0)], 1000)
        .unwrap();
    let duplicate_standardized = standardized.clone();
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![input_definition.clone(), definition.clone()],
        artifacts: vec![FittedArtifactBinding {
            artifact_id: artifact.artifact_id().into(),
            eligible_at_ms: artifact.eligible_at_ms(),
            fitted_output: fitted_output.clone(),
        }],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    let input = |time: i64, close: &str| {
        FeatureInputEvent::observation(FeatureEvaluationInput::new(
            "btc",
            time,
            time,
            FeatureMarketBar::complete(time, "10", "90", "1", close, "1", "1").unwrap(),
        ))
    };
    let mut evaluator = FeatureEngine::new(identity())
        .evaluator_with_artifacts(plan.clone(), &[artifact.clone()])
        .unwrap();
    let mut tampered = artifact.clone();
    tampered.eligible_at_ms += 1;
    assert!(
        FeatureEngine::new(identity())
            .evaluator_with_artifacts(plan, &[tampered])
            .is_err()
    );
    let duplicate_definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(11),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![local_close, duplicate_standardized],
        outputs: vec![FeatureOutput {
            name: "standardized-alt".into(),
            node_id: "standardized".into(),
        }],
    })
    .unwrap();
    let duplicate_fitted_output = FeatureReference {
        definition_hash: duplicate_definition.definition_hash().into(),
        node_id: "standardized".into(),
        output_name: "standardized-alt".into(),
    };
    let duplicate_plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![input_definition, definition, duplicate_definition],
        artifacts: vec![FittedArtifactBinding {
            artifact_id: artifact.artifact_id().into(),
            eligible_at_ms: artifact.eligible_at_ms(),
            fitted_output: duplicate_fitted_output,
        }],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    assert!(
        FeatureEngine::new(identity())
            .evaluator_with_artifacts(duplicate_plan, &[artifact.clone()])
            .is_err()
    );
    let observations = evaluator.evaluate_batch(&[input(100, "30")]).unwrap();
    let standardized = observations
        .iter()
        .find(|observation| observation.output_name == "standardized")
        .unwrap();
    assert!(matches!(
        standardized.value,
        FeatureObservationValue::Available {
            value,
            available_at_ms: 100,
        } if (value - 3.0).abs() < 1e-12
    ));
    assert_eq!(
        standardized.feature_reference.as_ref().unwrap().output_name,
        "standardized"
    );
    assert_eq!(artifact.created_at_ms(), 1000);
}
