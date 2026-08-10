use std::collections::BTreeMap;

use adaq_feature_engine::{
    DefinitionDraft, FeatureDefinition, FeatureEngineIdentity, FeatureEvaluationErrorCode,
    FeatureInput, FeatureMaterializationRequest, FeatureNode, FeatureObservation, FeatureOperator,
    FeatureOutput, FeaturePlan, FeaturePlanDraft, FeatureScope, FeatureSlot, FeatureSource,
    FeatureUnavailabilityReason, FittedArtifactBinding, MAX_CANONICAL_JSON_BYTES,
    MAX_EFFECTIVE_WARMUP_BARS, MarketField, ObservationRange, PlanLoadError,
};
use serde_json::json;
use uuid::Uuid;

fn identity() -> FeatureEngineIdentity {
    FeatureEngineIdentity::for_tests()
}

fn definition() -> FeatureDefinition {
    FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(1),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![FeatureNode {
            id: "return".into(),
            operator: FeatureOperator::BackwardSimpleReturn,
            scope: FeatureScope::TimeSeries,
            inputs: vec![FeatureInput::Market {
                field: "close".into(),
            }],
            parameters: BTreeMap::new(),
            warmup_bars: 1,
        }],
        outputs: vec![FeatureOutput {
            name: "return".into(),
            node_id: "return".into(),
        }],
    })
    .unwrap()
}

#[test]
fn definition_and_plan_identities_are_canonical_and_replayable() {
    let definition = definition();
    let definition_bytes = definition.to_json();
    assert_eq!(
        FeatureDefinition::load(&definition_bytes).unwrap(),
        definition
    );
    assert_eq!(definition_bytes, definition.to_json());

    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    let bytes = plan.to_json();
    assert_eq!(
        FeaturePlan::load_for_engine(&bytes, &identity()).unwrap(),
        plan
    );
    assert_eq!(plan.plan_hash().len(), 64);
    assert!(
        plan.plan_hash()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn plan_rejects_legacy_evidence_with_reset_required_error() {
    let legacy = br#"{"planSchemaVersion":"1.0.0"}"#;
    assert!(matches!(
        FeaturePlan::load_for_engine(legacy, &identity()),
        Err(PlanLoadError::ResetRequired { .. })
    ));
}

#[test]
fn plan_rejects_scope_cycles_and_bad_output_contracts() {
    let draft = DefinitionDraft {
        definition_id: Uuid::from_u128(2),
        revision: 1,
        scope: FeatureScope::CrossSectional,
        nodes: vec![
            FeatureNode {
                id: "a".into(),
                operator: FeatureOperator::CrossSectionalRank,
                scope: FeatureScope::CrossSectional,
                inputs: vec![FeatureInput::Node {
                    node_id: "b".into(),
                }],
                parameters: BTreeMap::new(),
                warmup_bars: 0,
            },
            FeatureNode {
                id: "b".into(),
                operator: FeatureOperator::CrossSectionalRank,
                scope: FeatureScope::CrossSectional,
                inputs: vec![FeatureInput::Node {
                    node_id: "a".into(),
                }],
                parameters: BTreeMap::new(),
                warmup_bars: 0,
            },
        ],
        outputs: vec![FeatureOutput {
            name: "Bad Name".into(),
            node_id: "a".into(),
        }],
    };
    let error = FeatureDefinition::freeze(draft).unwrap_err();
    assert!(error.codes().iter().any(|code| *code == "dependency-cycle"));
    assert!(
        error
            .codes()
            .iter()
            .any(|code| *code == "invalid-output-name")
    );
}

#[test]
fn observation_keeps_typed_unavailability_and_rejects_non_finite_values() {
    let unavailable = FeatureObservation::unavailable(
        "return",
        "BTC-USD",
        10,
        FeatureUnavailabilityReason::Warmup,
    )
    .unwrap();
    assert_eq!(
        unavailable.reason(),
        Some(FeatureUnavailabilityReason::Warmup)
    );
    let error = FeatureObservation::available("return", "BTC-USD", 10, f64::NAN, 10).unwrap_err();
    assert_eq!(error.code, FeatureEvaluationErrorCode::NonFiniteOutput);
    assert_eq!(error.instrument_id.as_deref(), Some("BTC-USD"));
    assert_eq!(error.observation_time_ms, Some(10));
}

#[test]
fn materialization_request_does_not_change_plan_identity() {
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition()],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    let request = FeatureMaterializationRequest::new(
        "user-1",
        plan.plan_hash(),
        "snapshot-1",
        "universe-1",
        ObservationRange {
            start_time_ms: 1,
            end_time_ms: 2,
        },
        BTreeMap::from([("period".into(), json!(5))]),
        42,
    )
    .unwrap();
    let plan_json = String::from_utf8(plan.to_json()).unwrap();
    assert!(!plan_json.contains("snapshotId"));
    assert!(request.request_hash().len() == 64);
}

#[test]
fn definition_and_plan_resource_limits_are_enforced() {
    let nodes = (0..257)
        .map(|index| FeatureNode {
            id: format!("node-{index}"),
            operator: FeatureOperator::CheckedArithmetic,
            scope: FeatureScope::Pointwise,
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            warmup_bars: 0,
        })
        .collect();
    let definition_error = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(3),
        revision: 1,
        scope: FeatureScope::Pointwise,
        nodes,
        outputs: vec![FeatureOutput {
            name: "output".into(),
            node_id: "node-0".into(),
        }],
    })
    .unwrap_err();
    assert!(
        definition_error
            .codes()
            .iter()
            .any(|code| *code == "too-many-definition-nodes")
    );

    let slots = (0..65)
        .map(|index| FeatureSlot {
            name: format!("output-{index}"),
            source: FeatureSource::Market {
                field: MarketField::Close,
            },
            warmup_bars: 0,
        })
        .collect();
    let plan_error = FeaturePlan::freeze(FeaturePlanDraft {
        slots,
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap_err();
    assert!(
        plan_error
            .codes()
            .iter()
            .any(|code| *code == "too-many-feature-outputs")
    );

    let oversized_definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(6),
        revision: 1,
        scope: FeatureScope::Pointwise,
        nodes: vec![FeatureNode {
            id: "large".into(),
            operator: FeatureOperator::CheckedArithmetic,
            scope: FeatureScope::Pointwise,
            inputs: Vec::new(),
            parameters: BTreeMap::from([(
                "payload".into(),
                json!("x".repeat(MAX_CANONICAL_JSON_BYTES)),
            )]),
            warmup_bars: 0,
        }],
        outputs: vec![FeatureOutput {
            name: "large".into(),
            node_id: "large".into(),
        }],
    })
    .unwrap_err();
    assert!(
        oversized_definition
            .codes()
            .iter()
            .any(|code| *code == "definition-json-too-large")
    );

    let warmup_definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(7),
        revision: 1,
        scope: FeatureScope::Pointwise,
        nodes: vec![FeatureNode {
            id: "warmup".into(),
            operator: FeatureOperator::CheckedArithmetic,
            scope: FeatureScope::Pointwise,
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            warmup_bars: MAX_EFFECTIVE_WARMUP_BARS + 1,
        }],
        outputs: vec![FeatureOutput {
            name: "warmup".into(),
            node_id: "warmup".into(),
        }],
    })
    .unwrap_err();
    assert!(
        warmup_definition
            .codes()
            .iter()
            .any(|code| *code == "effective-warmup-too-large")
    );
}

#[test]
fn plan_loader_rejects_noncanonical_and_mismatched_identity_documents() {
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition()],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap();
    let mut noncanonical = plan.to_json();
    noncanonical.push(b' ');
    assert!(matches!(
        FeaturePlan::load_for_engine(&noncanonical, &identity()),
        Err(PlanLoadError::NonCanonical)
    ));

    let mut document: serde_json::Value = serde_json::from_slice(&plan.to_json()).unwrap();
    document["planHash"] = serde_json::Value::String("a".repeat(64));
    let tampered =
        adaq_feature_engine::canonicalize_json(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(matches!(
        FeaturePlan::load_for_engine(&tampered, &identity()),
        Err(PlanLoadError::HashMismatch)
    ));

    let mut alternate_identity = identity();
    alternate_identity.target_triple = "other-target".into();
    assert!(matches!(
        FeaturePlan::load_for_engine(&plan.to_json(), &alternate_identity),
        Err(PlanLoadError::UnsupportedEngineIdentity)
    ));
}

#[test]
fn plan_rejects_unbound_artifact_inputs() {
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(5),
        revision: 1,
        scope: FeatureScope::Pointwise,
        nodes: vec![FeatureNode {
            id: "standardize".into(),
            operator: FeatureOperator::Standardization,
            scope: FeatureScope::Pointwise,
            inputs: vec![FeatureInput::Artifact {
                artifact_id: "missing".into(),
            }],
            parameters: BTreeMap::new(),
            warmup_bars: 0,
        }],
        outputs: vec![FeatureOutput {
            name: "standardized".into(),
            node_id: "standardize".into(),
        }],
    })
    .unwrap();
    let error = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition.clone()],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap_err();
    assert!(
        error
            .codes()
            .iter()
            .any(|code| *code == "unbound-artifact-input")
    );

    let bound = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        artifacts: vec![FittedArtifactBinding {
            artifact_id: "missing".into(),
            eligible_at_ms: 0,
        }],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    });
    assert!(bound.is_ok());
}

#[test]
fn plan_rejects_untyped_signal_provenance() {
    let error = FeaturePlan::freeze(FeaturePlanDraft {
        slots: vec![FeatureSlot {
            name: "return".into(),
            source: FeatureSource::Signal {
                dataset_id: "a".repeat(64),
                signal_name: "return".into(),
                snapshot_id: "snapshot".into(),
                instrument_id: "BTC-USD".into(),
                venue: "okx".into(),
                bar_interval: "1m".into(),
                contract: serde_json::Value::Null,
                producer_segments: vec![json!({})],
                artifact_provenance: json!({}),
                evidence_state: "unknown".into(),
                component_lock: vec![],
            },
            warmup_bars: 0,
        }],
        engine_identity: identity(),
        ..FeaturePlanDraft::default()
    })
    .unwrap_err();
    assert!(
        error
            .codes()
            .iter()
            .any(|code| *code == "invalid-signal-source")
    );
}

#[test]
fn definition_rejects_dependency_depth_beyond_the_contract() {
    let nodes = (0..65)
        .map(|index| FeatureNode {
            id: format!("node-{index}"),
            operator: FeatureOperator::CheckedArithmetic,
            scope: FeatureScope::Pointwise,
            inputs: if index == 0 {
                Vec::new()
            } else {
                vec![FeatureInput::Node {
                    node_id: format!("node-{}", index - 1),
                }]
            },
            parameters: BTreeMap::new(),
            warmup_bars: 0,
        })
        .collect();
    let error = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(4),
        revision: 1,
        scope: FeatureScope::Pointwise,
        nodes,
        outputs: vec![FeatureOutput {
            name: "output".into(),
            node_id: "node-64".into(),
        }],
    })
    .unwrap_err();
    assert!(
        error
            .codes()
            .iter()
            .any(|code| *code == "dependency-depth-too-large")
    );
}
