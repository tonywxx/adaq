use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use rusqlite::Connection;

use super::*;

const FACTOR_DECISION_ID: &str = "00000000-0000-4000-8000-000000000001";
const FACTOR_COMPONENT_ID: &str = "00000000-0000-4000-8000-000000000002";
const MODEL_COMPONENT_ID: &str = "00000000-0000-4000-8000-000000000003";

#[derive(Clone)]
struct FixtureSource {
    factor: ResolvedFactorInput,
    model: ResolvedModelInput,
    stale: Arc<AtomicBool>,
}

impl FixtureSource {
    fn new() -> Self {
        Self {
            factor: ResolvedFactorInput {
                decision_id: FACTOR_DECISION_ID.into(),
                decision_hash: hash('1'),
                candidate_hash: hash('2'),
                output_name: "momentum-score".into(),
                package_archive_sha256: hash('3'),
                package_wasm_sha256: hash('4'),
                component_id: FACTOR_COMPONENT_ID.into(),
                component_version: "1.0.0".into(),
                feature_plan_hash: hash('5'),
                context_hash: hash('6'),
                snapshot_id: "snapshot-1".into(),
                universe_id: "universe-1".into(),
                market: "crypto".into(),
                venue: "okx".into(),
            },
            model: ResolvedModelInput {
                qualification_report_id: hash('7'),
                decision_id: hash('8'),
                final_evaluation_report_id: hash('9'),
                artifact_sha256: hash('a'),
                transformation_sha256: hash('b'),
                package_archive_sha256: hash('c'),
                package_wasm_sha256: hash('d'),
                component_id: MODEL_COMPONENT_ID.into(),
                component_version: "1.0.0".into(),
                model_profile: "adaq:wasi-model@1".into(),
                exporter_id: "adaq:exporter@1".into(),
                sdk_version: "2.0.0".into(),
                abi_version: "2.0.0".into(),
                runtime_identity: "runtime-1".into(),
                input_slots: vec!["momentum-score".into()],
                output_name: "forecast".into(),
                target_id: "future-close-return".into(),
                target_horizon_bars: 5,
                forecast_contract: "forecast:continuous-future-close-return:native@1".into(),
                input_evidence_sha256: hash('e'),
            },
            stale: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl StrategyCandidateSource for FixtureSource {
    fn factor_inputs(&self, _user_id: &str) -> Result<Vec<ResolvedFactorInput>, String> {
        Ok(vec![self.factor.clone()])
    }

    fn model_inputs(&self, _user_id: &str) -> Result<Vec<ResolvedModelInput>, String> {
        Ok(vec![self.model.clone()])
    }

    fn resolve_factor(
        &self,
        _user_id: &str,
        binding: &FactorInputBinding,
    ) -> Result<ResolvedFactorInput, String> {
        if self.stale.load(Ordering::Acquire) {
            return Err("superseded".into());
        }
        let resolved = self.factor.clone();
        if resolved_matches_factor(&resolved, binding) {
            Ok(resolved)
        } else {
            Ok(resolved)
        }
    }

    fn resolve_model(
        &self,
        _user_id: &str,
        binding: &ModelInputBinding,
    ) -> Result<ResolvedModelInput, String> {
        if self.stale.load(Ordering::Acquire) {
            return Err("superseded".into());
        }
        let resolved = self.model.clone();
        if resolved_matches_model(&resolved, binding) {
            Ok(resolved)
        } else {
            Ok(resolved)
        }
    }
}

fn hash(character: char) -> String {
    std::iter::repeat_n(character, 64).collect()
}

fn source_slot(source: &FixtureSource) -> Vec<StrategyInputSlot> {
    vec![
        StrategyInputSlot {
            alias: "factor-score".into(),
            input_type: StrategyInputType::FactorScore,
            binding: StrategyInputBinding::Factor(FactorInputBinding {
                decision_id: source.factor.decision_id.clone(),
                decision_hash: source.factor.decision_hash.clone(),
                candidate_hash: source.factor.candidate_hash.clone(),
                output_name: source.factor.output_name.clone(),
                package_archive_sha256: source.factor.package_archive_sha256.clone(),
                package_wasm_sha256: source.factor.package_wasm_sha256.clone(),
                component_id: source.factor.component_id.clone(),
                component_version: source.factor.component_version.clone(),
            }),
        },
        StrategyInputSlot {
            alias: "forecast-signal".into(),
            input_type: StrategyInputType::ForecastSignal,
            binding: StrategyInputBinding::Model(ModelInputBinding {
                qualification_report_id: source.model.qualification_report_id.clone(),
                decision_id: source.model.decision_id.clone(),
                final_evaluation_report_id: source.model.final_evaluation_report_id.clone(),
                artifact_sha256: source.model.artifact_sha256.clone(),
                transformation_sha256: source.model.transformation_sha256.clone(),
                package_archive_sha256: source.model.package_archive_sha256.clone(),
                package_wasm_sha256: source.model.package_wasm_sha256.clone(),
                component_id: source.model.component_id.clone(),
                component_version: source.model.component_version.clone(),
                model_profile: source.model.model_profile.clone(),
                exporter_id: source.model.exporter_id.clone(),
                sdk_version: source.model.sdk_version.clone(),
                abi_version: source.model.abi_version.clone(),
                runtime_identity: source.model.runtime_identity.clone(),
                input_slots: source.model.input_slots.clone(),
                output_name: source.model.output_name.clone(),
                target_id: source.model.target_id.clone(),
                target_horizon_bars: source.model.target_horizon_bars,
                forecast_contract: source.model.forecast_contract.clone(),
            }),
        },
    ]
}

fn node(
    node_id: &str,
    operation: &str,
    input_aliases: &[&str],
    parameters: &[(&str, StrategyValue)],
    output_alias: &str,
) -> StrategyOperationNode {
    StrategyOperationNode {
        node_id: node_id.into(),
        operation: operation.into(),
        input_aliases: input_aliases.iter().map(|alias| (*alias).into()).collect(),
        parameters: parameters
            .iter()
            .map(|(name, value)| ((*name).into(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
        output_alias: output_alias.into(),
    }
}

fn draft(source: &FixtureSource, candidate_id: Option<String>) -> StrategyCandidateDraft {
    StrategyCandidateDraft {
        candidate_id,
        scope: StrategyScope::Portfolio,
        definition: StrategyDefinition {
            schema_version: STRATEGY_CANDIDATE_SCHEMA_VERSION.into(),
            catalog_version: STRATEGY_OPERATION_CATALOG_VERSION.into(),
            input_slots: source_slot(source),
            nodes: vec![
                node(
                    "combine-score",
                    "weighted-sum",
                    &["factor-score", "forecast-signal"],
                    &[("forecast-weight", StrategyValue::Decimal("0.7".into()))],
                    "combined-score",
                ),
                node(
                    "select-top",
                    "top-n",
                    &["combined-score"],
                    &[("top-n", StrategyValue::Integer(3))],
                    "selected-target",
                ),
                node(
                    "reserve-cash",
                    "cash-reserve",
                    &["selected-target"],
                    &[("cash-reserve", StrategyValue::Decimal("0.1".into()))],
                    "portfolio-target",
                ),
            ],
            output: StrategyOutputContract::PortfolioTarget {
                node_id: "reserve-cash".into(),
            },
        },
    }
}

fn single_draft(source: &FixtureSource, candidate_id: Option<String>) -> StrategyCandidateDraft {
    let mut draft = draft(source, candidate_id);
    draft.scope = StrategyScope::SingleInstrument;
    draft.definition.nodes.truncate(1);
    draft.definition.nodes[0].output_alias = "target-decision".into();
    draft.definition.output = StrategyOutputContract::TargetDecision {
        node_id: "combine-score".into(),
    };
    draft
}

fn store(source: FixtureSource) -> (StrategyCandidateStore, Arc<Mutex<Connection>>) {
    let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let store = StrategyCandidateStore::open(database.clone(), Arc::new(source)).unwrap();
    (store, database)
}

#[test]
fn publishes_append_only_revisions_and_recovers_from_the_same_database() {
    let source = FixtureSource::new();
    let (store, database) = store(source.clone());
    let first = store.preflight("alice", draft(&source, None)).unwrap();
    assert_eq!(first.status, StrategyAttemptStatus::ReadyToCreate);
    let candidate = store.create("alice", &first.attempt_id).unwrap();
    assert_eq!(candidate.revisions.len(), 1);
    let mut revised = draft(&source, Some(candidate.candidate_id.clone()));
    revised.definition.nodes[2]
        .parameters
        .insert("cash-reserve".into(), StrategyValue::Decimal("0".into()));
    let second = store.preflight("alice", revised).unwrap();
    assert_eq!(second.next_revision, 2);
    store.create("alice", &second.attempt_id).unwrap();
    let recovered = StrategyCandidateStore::open(database, Arc::new(source))
        .unwrap()
        .get("alice", &candidate.candidate_id)
        .unwrap();
    assert_eq!(recovered.revisions.len(), 2);
    assert_ne!(
        recovered.revisions[0].revision.revision_hash,
        recovered.revisions[1].revision.revision_hash
    );
}

#[test]
fn failed_preflight_is_retained_and_retry_does_not_publish_a_revision() {
    let source = FixtureSource::new();
    let (store, _) = store(source.clone());
    let mut invalid = draft(&source, None);
    invalid.definition.nodes[1].input_aliases = vec!["future-node-output".into()];
    let failed = store.preflight("alice", invalid).unwrap();
    assert_eq!(failed.status, StrategyAttemptStatus::Rejected);
    assert_eq!(store.list("alice").unwrap()[0].attempts.len(), 1);
    let retried = store.retry("alice", &failed.attempt_id).unwrap();
    assert_eq!(retried.status, StrategyAttemptStatus::Rejected);
    assert_ne!(retried.attempt_id, failed.attempt_id);
    assert!(store.list("alice").unwrap()[0].revisions.is_empty());
}

#[test]
fn scopes_are_supported_immutable_and_user_scoped() {
    let source = FixtureSource::new();
    let (store, _) = store(source.clone());
    let single = store
        .preflight("alice", single_draft(&source, None))
        .unwrap();
    assert_eq!(single.status, StrategyAttemptStatus::ReadyToCreate);
    let single_candidate = store.create("alice", &single.attempt_id).unwrap();
    assert_eq!(single_candidate.scope, StrategyScope::SingleInstrument);

    let changed_scope = store
        .preflight(
            "alice",
            draft(&source, Some(single_candidate.candidate_id.clone())),
        )
        .unwrap();
    assert_eq!(changed_scope.status, StrategyAttemptStatus::Rejected);
    assert_eq!(
        changed_scope.diagnostics[0].code,
        "strategy-scope-immutable"
    );

    let portfolio = store.preflight("alice", draft(&source, None)).unwrap();
    assert_ne!(portfolio.candidate_id, single_candidate.candidate_id);
    store.create("alice", &portfolio.attempt_id).unwrap();
    assert_eq!(store.list("alice").unwrap().len(), 2);
    assert!(store.list("bob").unwrap().is_empty());

    let foreign = store
        .preflight(
            "bob",
            draft(&source, Some(single_candidate.candidate_id.clone())),
        )
        .unwrap();
    assert_eq!(foreign.status, StrategyAttemptStatus::Rejected);
    assert_eq!(foreign.diagnostics[0].code, "strategy-candidate-not-owned");
}

#[test]
fn hash_mismatches_fail_closed_before_revision_publication() {
    let source = FixtureSource::new();
    let (store, _) = store(source.clone());
    let mut mismatched = draft(&source, None);
    let StrategyInputBinding::Factor(binding) = &mut mismatched.definition.input_slots[0].binding
    else {
        panic!("fixture factor slot changed kind");
    };
    binding.decision_hash = hash('f');
    let result = store.preflight("alice", mismatched).unwrap();
    assert_eq!(result.status, StrategyAttemptStatus::Rejected);
    assert_eq!(
        result.diagnostics[0].code,
        "strategy-factor-input-hash-mismatch"
    );
    assert!(store.list("alice").unwrap()[0].revisions.is_empty());
}

#[test]
fn stale_upstream_makes_the_frozen_revision_ineligible_without_deleting_it() {
    let source = FixtureSource::new();
    let (store, _) = store(source.clone());
    let first = store.preflight("alice", draft(&source, None)).unwrap();
    let candidate = store.create("alice", &first.attempt_id).unwrap();
    source.stale.store(true, Ordering::Release);
    let stale = store.get("alice", &candidate.candidate_id).unwrap();
    assert!(!stale.eligible);
    assert!(!stale.revisions[0].eligible);
    assert!(stale.revisions[0].stale_reason.is_some());
}
