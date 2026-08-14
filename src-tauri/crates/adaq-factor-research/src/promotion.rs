use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ContractError, EvaluationEvidenceState, FactorCandidate, FactorDatasetManifest,
    FactorEvaluationProtocol, FactorEvaluationReport, FactorLens, FactorPromotionDecision,
    FactorScope, MetricId, PromotedFactorLibrary, PromotionDecisionState, PromotionPolicy,
    ResearchEngineProvenance, ResearchLineage, ResearchTrial, content_hash, is_lower_kebab,
    is_sha256,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionPolicyDraft {
    pub policy_id: Uuid,
    pub revision: u64,
    pub required_lenses: Vec<FactorLens>,
    pub minimum_coverage: f64,
    pub minimum_samples: u64,
    pub maximum_holm_p_value: f64,
    pub require_subperiod_sign_consistency: bool,
    pub require_cost_aware_economic: bool,
}

impl PromotionPolicy {
    pub fn freeze(draft: PromotionPolicyDraft) -> Result<Self, ContractError> {
        let mut policy = Self {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            policy_id: draft.policy_id,
            revision: draft.revision,
            required_lenses: draft.required_lenses,
            minimum_coverage: draft.minimum_coverage,
            minimum_samples: draft.minimum_samples,
            maximum_holm_p_value: draft.maximum_holm_p_value,
            require_subperiod_sign_consistency: draft.require_subperiod_sign_consistency,
            require_cost_aware_economic: draft.require_cost_aware_economic,
            policy_hash: String::new(),
        };
        policy.validate_requirements()?;
        policy.policy_hash = {
            let content = policy.content();
            content_hash(&content)?
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn conservative_template(
        policy_id: Uuid,
        revision: u64,
        scope: FactorScope,
    ) -> Result<Self, ContractError> {
        Self::freeze(PromotionPolicyDraft {
            policy_id,
            revision,
            required_lenses: FactorLens::required(scope).to_vec(),
            minimum_coverage: 0.8,
            minimum_samples: 30,
            maximum_holm_p_value: 0.05,
            require_subperiod_sign_consistency: true,
            require_cost_aware_economic: true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionProtocolDraft {
    pub protocol_id: Uuid,
    pub user_id: Uuid,
    pub candidate_hash: String,
    pub output_name: String,
    pub family_id: Uuid,
    pub trial_id: Uuid,
    pub lineage_trial_ids: Vec<Uuid>,
    pub report_hashes: Vec<String>,
    pub policy_hash: String,
    pub engine_identity: ResearchEngineProvenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionProtocol {
    pub schema_version: String,
    pub protocol_id: Uuid,
    pub user_id: Uuid,
    pub candidate_hash: String,
    pub output_name: String,
    pub family_id: Uuid,
    pub trial_id: Uuid,
    pub lineage_trial_ids: Vec<Uuid>,
    pub lineage_hash: String,
    pub report_hashes: Vec<String>,
    pub policy_hash: String,
    pub engine_identity: ResearchEngineProvenance,
    pub protocol_hash: String,
}

impl PromotionProtocol {
    pub fn freeze(
        draft: PromotionProtocolDraft,
        lineage_hash: String,
    ) -> Result<Self, ContractError> {
        let mut protocol = Self {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            protocol_id: draft.protocol_id,
            user_id: draft.user_id,
            candidate_hash: draft.candidate_hash,
            output_name: draft.output_name,
            family_id: draft.family_id,
            trial_id: draft.trial_id,
            lineage_trial_ids: draft.lineage_trial_ids,
            lineage_hash,
            report_hashes: draft.report_hashes,
            policy_hash: draft.policy_hash,
            engine_identity: draft.engine_identity,
            protocol_hash: String::new(),
        };
        protocol.validate_without_hash()?;
        protocol.protocol_hash = {
            let content = protocol.content();
            content_hash(&content)?
        };
        protocol.validate()?;
        Ok(protocol)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        self.validate_without_hash()?;
        if !is_sha256(&self.protocol_hash) || self.protocol_hash != content_hash(&self.content())? {
            return Err(ContractError::HashMismatch);
        }
        Ok(())
    }

    fn validate_without_hash(&self) -> Result<(), ContractError> {
        if self.schema_version != crate::FACTOR_RESEARCH_SCHEMA_VERSION
            || self.protocol_id.is_nil()
            || self.user_id.is_nil()
            || !is_sha256(&self.candidate_hash)
            || !is_lower_kebab(&self.output_name)
            || self.family_id.is_nil()
            || self.trial_id.is_nil()
            || self.lineage_trial_ids.is_empty()
            || self
                .lineage_trial_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || !self.lineage_trial_ids.contains(&self.trial_id)
            || !is_sha256(&self.lineage_hash)
            || self.report_hashes.is_empty()
            || self.report_hashes.windows(2).any(|pair| pair[0] >= pair[1])
            || self.report_hashes.iter().any(|hash| !is_sha256(hash))
            || !is_sha256(&self.policy_hash)
        {
            return Err(ContractError::Invalid(
                "Promotion Protocol identity or lineage is invalid".into(),
            ));
        }
        self.engine_identity.validate()
    }

    pub(crate) fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            schema_version: &'a str,
            protocol_id: Uuid,
            user_id: Uuid,
            candidate_hash: &'a str,
            output_name: &'a str,
            family_id: Uuid,
            trial_id: Uuid,
            lineage_trial_ids: &'a [Uuid],
            lineage_hash: &'a str,
            report_hashes: &'a [String],
            policy_hash: &'a str,
            engine_identity: &'a ResearchEngineProvenance,
        }
        Content {
            schema_version: &self.schema_version,
            protocol_id: self.protocol_id,
            user_id: self.user_id,
            candidate_hash: &self.candidate_hash,
            output_name: &self.output_name,
            family_id: self.family_id,
            trial_id: self.trial_id,
            lineage_trial_ids: &self.lineage_trial_ids,
            lineage_hash: &self.lineage_hash,
            report_hashes: &self.report_hashes,
            policy_hash: &self.policy_hash,
            engine_identity: &self.engine_identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactorDatasetStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromotionGate {
    CompleteLineage,
    OutOfSampleReport,
    RequiredLenses,
    MinimumCoverage,
    MinimumSamples,
    HolmAdjustedSignificance,
    SubperiodSignConsistency,
    CostAwareOutcome,
    CompleteProvenance,
    CompleteSourceProvenance,
    DeterministicExecution,
    AbiV2Expressible,
    Buildable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionGateResult {
    pub gate: PromotionGate,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionEligibility {
    candidate_hash: String,
    output_name: String,
    promotion_protocol_hash: String,
    gates: Vec<PromotionGateResult>,
    research_validated: bool,
    component_eligible: bool,
    #[serde(skip)]
    verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentEligibilityEvidence {
    pub deterministic_execution: bool,
    pub complete_source_provenance: bool,
    pub abi_v2_expressible: bool,
    pub buildable: bool,
}

pub struct PromotionEvidence<'a> {
    pub candidate: &'a FactorCandidate,
    pub dataset: &'a FactorDatasetManifest,
    pub dataset_status: FactorDatasetStatus,
    pub evaluation_protocol: &'a FactorEvaluationProtocol,
    pub reports: &'a [FactorEvaluationReport],
    pub policy: &'a PromotionPolicy,
    pub lineage: &'a ResearchLineage,
    pub promotion_protocol: &'a PromotionProtocol,
    pub component: ComponentEligibilityEvidence,
}

impl PromotionEligibility {
    pub fn candidate_hash(&self) -> &str {
        &self.candidate_hash
    }

    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    pub const fn research_validated(&self) -> bool {
        self.research_validated
    }

    pub const fn component_eligible(&self) -> bool {
        self.component_eligible
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if !self.verified
            || !is_sha256(&self.candidate_hash)
            || !is_lower_kebab(&self.output_name)
            || !is_sha256(&self.promotion_protocol_hash)
            || self
                .gates
                .iter()
                .enumerate()
                .any(|(index, gate)| self.gates[..index].contains(gate))
            || (self.component_eligible && !self.research_validated)
        {
            return Err(ContractError::Invalid(
                "Promotion eligibility identity or gate state is invalid".into(),
            ));
        }
        Ok(())
    }

    pub fn check(evidence: PromotionEvidence<'_>) -> Result<Self, ContractError> {
        Self::check_with_trial(evidence, None)
    }

    pub fn check_with_trial(
        evidence: PromotionEvidence<'_>,
        trial: Option<&ResearchTrial>,
    ) -> Result<Self, ContractError> {
        evidence.candidate.validate()?;
        evidence.dataset.validate()?;
        evidence.evaluation_protocol.validate()?;
        evidence.policy.validate()?;
        evidence.lineage.validate()?;
        evidence.promotion_protocol.validate()?;
        if !evidence.lineage.is_registry_derived() {
            return Err(ContractError::Invalid(
                "Promotion requires registry-derived complete lineage".into(),
            ));
        }
        if evidence.dataset_status != FactorDatasetStatus::Completed
            || evidence.dataset.candidate_hash != evidence.candidate.candidate_hash
            || evidence.dataset.dataset_id != evidence.evaluation_protocol.factor_dataset_id
            || evidence.evaluation_protocol.user_id != evidence.promotion_protocol.user_id
            || evidence.evaluation_protocol.family_id != evidence.promotion_protocol.family_id
            || evidence.evaluation_protocol.trial_id != evidence.promotion_protocol.trial_id
            || evidence.lineage.root_trial_id != evidence.promotion_protocol.trial_id
            || evidence.lineage.trial_ids != evidence.promotion_protocol.lineage_trial_ids
            || evidence.lineage.lineage_hash != evidence.promotion_protocol.lineage_hash
            || evidence.evaluation_protocol.output_name != evidence.promotion_protocol.output_name
            || evidence.evaluation_protocol.engine_identity
                != evidence.promotion_protocol.engine_identity
            || evidence.policy.policy_hash != evidence.promotion_protocol.policy_hash
        {
            return Err(ContractError::Invalid(
                "Promotion evidence identities do not agree".into(),
            ));
        }
        if !evidence
            .dataset
            .output_names
            .iter()
            .any(|name| name == &evidence.promotion_protocol.output_name)
            || evidence
                .candidate
                .outputs
                .iter()
                .all(|output| output.name != evidence.promotion_protocol.output_name)
        {
            return Err(ContractError::Invalid(
                "Promotion output is not an exact Dataset and Candidate output".into(),
            ));
        }
        if let Some(trial) = trial
            && (trial.trial_id != evidence.promotion_protocol.trial_id
                || trial.family_id != evidence.promotion_protocol.family_id
                || trial.candidate_hash != evidence.candidate.candidate_hash
                || trial.report_hash.as_deref().is_none_or(|hash| {
                    !evidence
                        .promotion_protocol
                        .report_hashes
                        .iter()
                        .any(|item| item == hash)
                }))
        {
            return Err(ContractError::Invalid(
                "Promotion Trial is not bound to the exact Promotion Protocol".into(),
            ));
        }

        let reports = evidence
            .reports
            .iter()
            .filter(|report| {
                evidence
                    .promotion_protocol
                    .report_hashes
                    .iter()
                    .any(|hash| hash == &report.report_hash)
            })
            .collect::<Vec<_>>();
        if reports.len() != evidence.promotion_protocol.report_hashes.len() {
            return Err(ContractError::Invalid(
                "Promotion Protocol reports are not available".into(),
            ));
        }
        for report in &reports {
            report.validate()?;
            if report.protocol_hash != evidence.evaluation_protocol.protocol_hash
                || report.factor_dataset_id != evidence.dataset.dataset_id
                || report.output_name != evidence.promotion_protocol.output_name
                || report.market_data_snapshot_id
                    != evidence.evaluation_protocol.market_data_snapshot_id
                || report.point_in_time_universe_id
                    != evidence.evaluation_protocol.point_in_time_universe_id
            {
                return Err(ContractError::Invalid(
                    "Promotion Report is not bound to the frozen Protocol".into(),
                ));
            }
        }

        let lineage = gate(
            PromotionGate::CompleteLineage,
            evidence
                .promotion_protocol
                .lineage_trial_ids
                .contains(&evidence.promotion_protocol.trial_id)
                && !evidence.promotion_protocol.lineage_trial_ids.is_empty(),
        );
        let out_of_sample = gate(
            PromotionGate::OutOfSampleReport,
            reports
                .iter()
                .any(|report| report.evidence_state == EvaluationEvidenceState::OutOfSample),
        );
        let policy_report = reports.iter().any(|report| {
            report.evidence_state == EvaluationEvidenceState::OutOfSample
                && report_satisfies_policy(report, evidence.policy, trial)
        });
        let lineage_passed = lineage.passed;
        let out_of_sample_passed = out_of_sample.passed;
        let gates = vec![
            lineage.clone(),
            out_of_sample.clone(),
            gate(
                PromotionGate::RequiredLenses,
                reports
                    .iter()
                    .any(|report| has_required_lenses(report, evidence.policy)),
            ),
            gate(
                PromotionGate::MinimumCoverage,
                reports.iter().any(|report| {
                    has_threshold(report, MetricId::Coverage, |value| {
                        value >= evidence.policy.minimum_coverage
                    })
                }),
            ),
            gate(
                PromotionGate::MinimumSamples,
                reports.iter().any(|report| {
                    has_threshold(report, MetricId::SampleCount, |value| {
                        value >= evidence.policy.minimum_samples as f64
                    })
                }),
            ),
            gate(
                PromotionGate::HolmAdjustedSignificance,
                reports.iter().any(|report| {
                    has_threshold(report, MetricId::HolmAdjusted, |value| {
                        value <= evidence.policy.maximum_holm_p_value
                    })
                }) || trial.is_some_and(|trial| {
                    trial
                        .holm_adjusted
                        .as_ref()
                        .and_then(crate::MetricObservation::value)
                        .is_some_and(|value| value <= evidence.policy.maximum_holm_p_value)
                }),
            ),
            gate(
                PromotionGate::SubperiodSignConsistency,
                !evidence.policy.require_subperiod_sign_consistency
                    || reports.iter().any(|report| {
                        has_threshold(report, MetricId::Stability, |value| value >= 1.0)
                    }),
            ),
            gate(
                PromotionGate::CostAwareOutcome,
                !evidence.policy.require_cost_aware_economic
                    || reports.iter().any(|report| has_cost_aware_outcome(report)),
            ),
        ];
        let provenance = gate(
            PromotionGate::CompleteProvenance,
            reports.iter().all(|report| {
                !report.input_identities.is_empty()
                    && report
                        .input_identities
                        .iter()
                        .all(|identity| !identity.trim().is_empty())
                    && evidence.dataset.engine_identity
                        == evidence.evaluation_protocol.engine_identity
            }),
        );
        let research_validated =
            lineage_passed && out_of_sample_passed && policy_report && provenance.passed;
        let component_gates = [
            gate(
                PromotionGate::CompleteSourceProvenance,
                evidence.component.complete_source_provenance,
            ),
            gate(
                PromotionGate::DeterministicExecution,
                evidence.component.deterministic_execution,
            ),
            gate(
                PromotionGate::AbiV2Expressible,
                evidence.component.abi_v2_expressible,
            ),
            gate(PromotionGate::Buildable, evidence.component.buildable),
        ];
        let component_eligible =
            research_validated && component_gates.iter().all(|gate| gate.passed);
        let mut gates = gates;
        gates.push(provenance);
        gates.extend(component_gates);
        let eligibility = Self {
            candidate_hash: evidence.candidate.candidate_hash.clone(),
            output_name: evidence.promotion_protocol.output_name.clone(),
            promotion_protocol_hash: evidence.promotion_protocol.protocol_hash.clone(),
            gates,
            research_validated,
            component_eligible,
            verified: true,
        };
        eligibility.validate()?;
        Ok(eligibility)
    }

    pub fn gate(&self, gate: PromotionGate) -> bool {
        self.gates
            .iter()
            .find(|result| result.gate == gate)
            .is_some_and(|result| result.passed)
    }

    pub fn gates(&self) -> &[PromotionGateResult] {
        &self.gates
    }
}

fn gate(gate: PromotionGate, passed: bool) -> PromotionGateResult {
    PromotionGateResult { gate, passed }
}

fn has_required_lenses(report: &FactorEvaluationReport, policy: &PromotionPolicy) -> bool {
    policy.required_lenses.iter().all(|lens| {
        report
            .metrics
            .iter()
            .any(|metric| metric.lens == *lens && metric.observation.value().is_some())
    })
}

fn has_cost_aware_outcome(report: &FactorEvaluationReport) -> bool {
    report.metrics.iter().any(|record| {
        record.metric == MetricId::Economic
            && record.lens == FactorLens::Economic
            && matches!(record.variant.as_str(), "top-only" | "top-minus-bottom")
            && record.observation.value().is_some()
    })
}

fn has_threshold(
    report: &FactorEvaluationReport,
    metric: MetricId,
    predicate: impl Fn(f64) -> bool,
) -> bool {
    report
        .metrics
        .iter()
        .any(|record| record.metric == metric && record.observation.value().is_some_and(&predicate))
}

fn report_satisfies_policy(
    report: &FactorEvaluationReport,
    policy: &PromotionPolicy,
    trial: Option<&ResearchTrial>,
) -> bool {
    has_required_lenses(report, policy)
        && has_threshold(report, MetricId::Coverage, |value| {
            value >= policy.minimum_coverage
        })
        && has_threshold(report, MetricId::SampleCount, |value| {
            value >= policy.minimum_samples as f64
        })
        && (has_threshold(report, MetricId::HolmAdjusted, |value| {
            value <= policy.maximum_holm_p_value
        }) || trial.is_some_and(|trial| {
            trial
                .holm_adjusted
                .as_ref()
                .and_then(crate::MetricObservation::value)
                .is_some_and(|value| value <= policy.maximum_holm_p_value)
        }))
        && (!policy.require_subperiod_sign_consistency
            || has_threshold(report, MetricId::Stability, |value| value >= 1.0))
        && (!policy.require_cost_aware_economic || has_cost_aware_outcome(report))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionDecisionDraft {
    pub decision_id: Uuid,
    pub user_id: Uuid,
    pub candidate_hash: String,
    pub output_name: String,
    pub state: PromotionDecisionState,
    pub report_hashes: Vec<String>,
    pub policy_hash: String,
    pub evidence_state: EvaluationEvidenceState,
    pub supersedes: Option<Uuid>,
}

impl FactorPromotionDecision {
    pub fn freeze(draft: PromotionDecisionDraft) -> Result<Self, ContractError> {
        let mut decision = Self {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            decision_id: draft.decision_id,
            user_id: draft.user_id,
            candidate_hash: draft.candidate_hash,
            output_name: draft.output_name,
            state: draft.state,
            report_hashes: draft.report_hashes,
            policy_hash: draft.policy_hash,
            evidence_state: draft.evidence_state,
            supersedes: draft.supersedes,
            decision_hash: String::new(),
        };
        decision.decision_hash = {
            let content = decision.content();
            content_hash(&content)?
        };
        decision.validate()?;
        Ok(decision)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionDecisionRecord {
    pub decision: FactorPromotionDecision,
    pub promotion_protocol_hash: String,
    pub eligibility_gates: Vec<PromotionGateResult>,
    pub component: ComponentEligibilityEvidence,
}

impl PromotionDecisionRecord {
    pub fn validate(&self) -> Result<(), ContractError> {
        self.decision.validate()?;
        if !is_sha256(&self.promotion_protocol_hash) {
            return Err(ContractError::Invalid(
                "Promotion Decision Protocol identity is invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionError {
    Contract(ContractError),
    DuplicateIdentity,
    Unauthorized,
    SupersededDecisionNotFound,
    SupersessionRequired,
    EligibilityRequired,
}

impl std::fmt::Display for DecisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::DuplicateIdentity => formatter.write_str("Promotion Decision identity exists"),
            Self::Unauthorized => formatter.write_str("Promotion Decisions are user-scoped"),
            Self::SupersededDecisionNotFound => {
                formatter.write_str("superseded Decision was not found")
            }
            Self::SupersessionRequired => {
                formatter.write_str("a later Decision must supersede the current Decision")
            }
            Self::EligibilityRequired => {
                formatter.write_str("system eligibility must pass before a positive Decision")
            }
        }
    }
}

impl std::error::Error for DecisionError {}

impl From<ContractError> for DecisionError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

#[derive(Debug, Clone, Default)]
pub struct PromotionDecisionLedger {
    records: BTreeMap<Uuid, PromotionDecisionRecord>,
}

impl PromotionDecisionLedger {
    pub fn append(
        &mut self,
        decision: FactorPromotionDecision,
        protocol: &PromotionProtocol,
        eligibility: &PromotionEligibility,
    ) -> Result<PromotionDecisionRecord, DecisionError> {
        decision.validate()?;
        protocol.validate()?;
        eligibility.validate()?;
        if self.records.contains_key(&decision.decision_id) {
            return Err(DecisionError::DuplicateIdentity);
        }
        if decision.user_id != protocol.user_id
            || decision.candidate_hash != protocol.candidate_hash
            || decision.output_name != protocol.output_name
            || decision.report_hashes != protocol.report_hashes
            || decision.policy_hash != protocol.policy_hash
            || eligibility.promotion_protocol_hash != protocol.protocol_hash
            || eligibility.candidate_hash != decision.candidate_hash
            || eligibility.output_name != decision.output_name
        {
            return Err(DecisionError::Contract(ContractError::Invalid(
                "Decision and Promotion Protocol identities differ".into(),
            )));
        }
        match decision.state {
            PromotionDecisionState::ResearchValidated if !eligibility.research_validated => {
                return Err(DecisionError::EligibilityRequired);
            }
            PromotionDecisionState::ComponentEligible if !eligibility.component_eligible => {
                return Err(DecisionError::EligibilityRequired);
            }
            _ => {}
        }
        let current = self.current(
            decision.user_id,
            &decision.candidate_hash,
            &decision.output_name,
        );
        match (current, decision.supersedes) {
            (Some(current), Some(supersedes)) if current.decision.decision_id == supersedes => {}
            (Some(_), None) => return Err(DecisionError::SupersessionRequired),
            (Some(_), Some(_)) => return Err(DecisionError::SupersededDecisionNotFound),
            (None, Some(supersedes)) if !self.records.contains_key(&supersedes) => {
                return Err(DecisionError::SupersededDecisionNotFound);
            }
            (None, Some(_)) => return Err(DecisionError::SupersededDecisionNotFound),
            (None, None) => {}
        }
        if let Some(supersedes) = decision.supersedes {
            let previous = self
                .records
                .get(&supersedes)
                .ok_or(DecisionError::SupersededDecisionNotFound)?;
            if previous.decision.user_id != decision.user_id
                || previous.decision.candidate_hash != decision.candidate_hash
                || previous.decision.output_name != decision.output_name
            {
                return Err(DecisionError::Unauthorized);
            }
        }
        let record = PromotionDecisionRecord {
            decision,
            promotion_protocol_hash: protocol.protocol_hash.clone(),
            eligibility_gates: eligibility.gates().to_vec(),
            component: ComponentEligibilityEvidence::default(),
        };
        record.validate()?;
        self.records
            .insert(record.decision.decision_id, record.clone());
        Ok(record)
    }

    pub fn decisions(&self, user_id: Uuid) -> Vec<PromotionDecisionRecord> {
        self.records
            .values()
            .filter(|record| record.decision.user_id == user_id)
            .cloned()
            .collect()
    }

    pub fn current(
        &self,
        user_id: Uuid,
        candidate_hash: &str,
        output_name: &str,
    ) -> Option<PromotionDecisionRecord> {
        let superseded = self
            .records
            .values()
            .filter_map(|record| record.decision.supersedes)
            .collect::<BTreeSet<_>>();
        self.records
            .values()
            .filter(|record| {
                record.decision.user_id == user_id
                    && record.decision.candidate_hash == candidate_hash
                    && record.decision.output_name == output_name
                    && !superseded.contains(&record.decision.decision_id)
            })
            .max_by_key(|record| record.decision.decision_id)
            .cloned()
    }

    pub fn library(&self, user_id: Uuid) -> Result<PromotedFactorLibrary, ContractError> {
        let mut entries = self
            .decisions(user_id)
            .into_iter()
            .filter(|record| {
                matches!(
                    record.decision.state,
                    PromotionDecisionState::ResearchValidated
                        | PromotionDecisionState::ComponentEligible
                ) && self
                    .current(
                        user_id,
                        &record.decision.candidate_hash,
                        &record.decision.output_name,
                    )
                    .is_some_and(|current| {
                        current.decision.decision_id == record.decision.decision_id
                    })
            })
            .map(|record| crate::PromotedFactorLibraryEntry {
                candidate_hash: record.decision.candidate_hash,
                output_name: record.decision.output_name,
                decision_id: record.decision.decision_id,
                report_hashes: record.decision.report_hashes,
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.candidate_hash
                .cmp(&right.candidate_hash)
                .then_with(|| left.output_name.cmp(&right.output_name))
        });
        let mut library = PromotedFactorLibrary {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            user_id,
            entries,
            library_hash: String::new(),
        };
        library.library_hash = {
            let content = library.content();
            content_hash(&content)?
        };
        library.validate()?;
        Ok(library)
    }

    pub fn m14_component_outputs(&self, user_id: Uuid, candidate: &FactorCandidate) -> Vec<String> {
        if !matches!(
            candidate.source,
            crate::FactorCandidateSource::Custom { .. }
        ) {
            return Vec::new();
        }
        let outputs = candidate
            .outputs
            .iter()
            .filter(|output| {
                self.current(user_id, &candidate.candidate_hash, &output.name)
                    .is_some_and(|record| {
                        record.decision.state == PromotionDecisionState::ComponentEligible
                    })
            })
            .map(|output| output.name.clone())
            .collect::<Vec<_>>();
        if outputs.len() == candidate.outputs.len() {
            outputs
        } else {
            Vec::new()
        }
    }

    pub fn m12_eligibility(&self, input: M12EligibilityInput<'_>) -> M12Eligibility {
        let current = self.current(
            input.user_id,
            &input.promotion_protocol.candidate_hash,
            &input.promotion_protocol.output_name,
        );
        let gates = current
            .as_ref()
            .map(|record| record.eligibility_gates.clone())
            .unwrap_or_default();
        let valid = input.dataset_status == FactorDatasetStatus::Completed
            && input.dataset.validate().is_ok()
            && input.evaluation_protocol.validate().is_ok()
            && input.report.validate().is_ok()
            && input.policy.validate().is_ok()
            && input.promotion_protocol.validate().is_ok()
            && input
                .dataset
                .output_names
                .contains(&input.promotion_protocol.output_name)
            && input.dataset.dataset_id == input.evaluation_protocol.factor_dataset_id
            && input.dataset.candidate_hash == input.promotion_protocol.candidate_hash
            && input.evaluation_protocol.user_id == input.user_id
            && input.evaluation_protocol.family_id == input.promotion_protocol.family_id
            && input.evaluation_protocol.trial_id == input.promotion_protocol.trial_id
            && input
                .promotion_protocol
                .lineage_trial_ids
                .contains(&input.evaluation_protocol.trial_id)
            && input.evaluation_protocol.output_name == input.promotion_protocol.output_name
            && input.report.protocol_hash == input.evaluation_protocol.protocol_hash
            && input.report.factor_dataset_id == input.dataset.dataset_id
            && input.report.output_name == input.promotion_protocol.output_name
            && input.report.market_data_snapshot_id
                == input.evaluation_protocol.market_data_snapshot_id
            && input.report.point_in_time_universe_id
                == input.evaluation_protocol.point_in_time_universe_id
            && input.report.report_hash == input.report_hash
            && input
                .promotion_protocol
                .report_hashes
                .contains(&input.report.report_hash)
            && input.promotion_protocol.policy_hash == input.policy.policy_hash
            && input.evaluation_protocol.engine_identity
                == input.promotion_protocol.engine_identity
            && input.dataset.engine_identity == input.evaluation_protocol.engine_identity
            && input.report.evidence_state == EvaluationEvidenceState::OutOfSample
            && current.as_ref().is_some_and(|record| {
                matches!(
                    record.decision.state,
                    PromotionDecisionState::ResearchValidated
                        | PromotionDecisionState::ComponentEligible
                ) && record.promotion_protocol_hash == input.promotion_protocol.protocol_hash
            });
        M12Eligibility {
            eligible: valid,
            reason: (!valid)
                .then_some("completed output lacks a current frozen promotion evidence set"),
            gates,
        }
    }
}

pub struct M12EligibilityInput<'a> {
    pub user_id: Uuid,
    pub dataset_status: FactorDatasetStatus,
    pub dataset: &'a FactorDatasetManifest,
    pub evaluation_protocol: &'a FactorEvaluationProtocol,
    pub report: &'a FactorEvaluationReport,
    pub report_hash: String,
    pub policy: &'a PromotionPolicy,
    pub promotion_protocol: &'a PromotionProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct M12Eligibility {
    pub eligible: bool,
    pub reason: Option<&'static str>,
    pub gates: Vec<PromotionGateResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MetricRecord;

    fn engine() -> crate::ResearchEngineProvenance {
        crate::ResearchEngineProvenance {
            engine_id: "native".into(),
            engine_version: "1".into(),
            adapter: "native".into(),
            target_triple: "test".into(),
            build_id: "build".into(),
            environment: BTreeMap::new(),
            parameters: BTreeMap::new(),
            input_identities: vec!["input".into()],
        }
    }

    fn context() -> crate::FactorMarketContext {
        crate::FactorMarketContext {
            venue: "venue".into(),
            asset_class: "asset".into(),
            bar_interval: "1h".into(),
            price_basis: "close".into(),
            valuation_currency: "USD".into(),
            point_in_time_universe_id: "universe".into(),
        }
    }

    fn candidate() -> crate::FactorCandidate {
        crate::FactorCandidate::freeze(crate::FactorCandidateDraft {
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: FactorScope::TimeSeries,
            feature_slots: vec![crate::FactorFeatureSlot {
                name: "close".into(),
            }],
            parameters: Vec::new(),
            outputs: vec![crate::FactorOutput {
                name: "value".into(),
            }],
            source: crate::FactorCandidateSource::Declarative {
                definition: crate::DeclarativeFactorDefinition {
                    feature_plan_hash: "a".repeat(64),
                    operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                        .into(),
                    outputs: vec![crate::DeclarativeFactorOutputBinding {
                        output_name: "value".into(),
                        feature_slot: "close".into(),
                    }],
                },
            },
        })
        .unwrap()
    }

    fn custom_candidate() -> crate::FactorCandidate {
        crate::FactorCandidate::freeze(crate::FactorCandidateDraft {
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: FactorScope::TimeSeries,
            feature_slots: vec![crate::FactorFeatureSlot {
                name: "close".into(),
            }],
            parameters: Vec::new(),
            outputs: vec![
                crate::FactorOutput {
                    name: "first".into(),
                },
                crate::FactorOutput {
                    name: "second".into(),
                },
            ],
            source: crate::FactorCandidateSource::Custom {
                build: crate::CandidateBuildProvenance {
                    attempt_id: Uuid::new_v4(),
                    source_sha256: "a".repeat(64),
                    sdk_version: "0.1.0".into(),
                    abi_version: crate::FACTOR_ABI_VERSION.into(),
                    toolchain: "stable".into(),
                    compiler: "rustc".into(),
                    target: "wasm32-unknown-unknown".into(),
                    commands: vec!["cargo component build".into()],
                    environment: BTreeMap::new(),
                    resource_policy: crate::FactorResourcePolicy {
                        fuel_per_call: 1,
                        memory_bytes: 1,
                    },
                    diagnostic_log_sha256: None,
                    package_sha256: "b".repeat(64),
                },
            },
        })
        .unwrap()
    }

    fn evaluation_protocol(
        user_id: Uuid,
        family_id: Uuid,
        trial_id: Uuid,
        candidate: &crate::FactorCandidate,
    ) -> crate::FactorEvaluationProtocol {
        crate::FactorEvaluationProtocol::freeze(crate::FactorEvaluationProtocolDraft {
            protocol_id: Uuid::new_v4(),
            user_id,
            factor_dataset_id: "dataset".into(),
            feature_dataset_id: "feature".into(),
            feature_plan_hash: "a".repeat(64),
            market_data_snapshot_id: "snapshot".into(),
            point_in_time_universe_id: "universe".into(),
            point_in_time_universe: vec!["asset".into()],
            output_name: candidate.outputs[0].name.clone(),
            scope: FactorScope::TimeSeries,
            target: crate::FactorTarget::FutureCloseReturn,
            horizon_bars: vec![1],
            market_context: context(),
            engine_identity: engine(),
            orientation: crate::FactorOrientation::Positive,
            windows: vec![crate::EvaluationWindow {
                fold_id: "fold-1".into(),
                selection: crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 10,
                },
                evaluation: crate::ObservationRange {
                    start_time_ms: 20,
                    end_time_ms: 30,
                },
                training: Some(crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 10,
                }),
                fitting: Some(crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 10,
                }),
                normalization: Some(crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 10,
                }),
                target_construction: Some(crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 10,
                }),
            }],
            purge_bars: 0,
            embargo_bars: 0,
            lenses: vec![FactorLens::Temporal, FactorLens::Economic],
            nuisance_feature_names: Vec::new(),
            regime: None,
            economic: crate::EconomicAssumptions {
                rebalance_every_bars: 1,
                fee_bps: 0.0,
                slippage_bps: 0.0,
                long_short: false,
            },
            family_id,
            trial_id,
            seed: 1,
        })
        .unwrap()
    }

    fn report(protocol: &crate::FactorEvaluationProtocol) -> crate::FactorEvaluationReport {
        let observation = |value| crate::MetricObservation::available(value, 100).unwrap();
        crate::FactorEvaluationReport::freeze(crate::FactorEvaluationReport {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            report_id: Uuid::new_v4(),
            protocol_hash: protocol.protocol_hash.clone(),
            factor_dataset_id: protocol.factor_dataset_id.clone(),
            output_name: protocol.output_name.clone(),
            scope: protocol.scope,
            target: protocol.target,
            market_data_snapshot_id: protocol.market_data_snapshot_id.clone(),
            point_in_time_universe_id: protocol.point_in_time_universe_id.clone(),
            market_context: protocol.market_context.clone(),
            evidence_state: EvaluationEvidenceState::OutOfSample,
            metrics: vec![
                MetricRecord {
                    fold_id: "fold-1".into(),
                    variant: "overall".into(),
                    horizon_bars: 1,
                    output_name: protocol.output_name.clone(),
                    lens: FactorLens::Temporal,
                    metric: MetricId::Coverage,
                    observation: observation(1.0),
                },
                MetricRecord {
                    fold_id: "fold-1".into(),
                    variant: "overall".into(),
                    horizon_bars: 1,
                    output_name: protocol.output_name.clone(),
                    lens: FactorLens::Temporal,
                    metric: MetricId::SampleCount,
                    observation: observation(100.0),
                },
                MetricRecord {
                    fold_id: "fold-1".into(),
                    variant: "overall".into(),
                    horizon_bars: 1,
                    output_name: protocol.output_name.clone(),
                    lens: FactorLens::Temporal,
                    metric: MetricId::HolmAdjusted,
                    observation: observation(0.01),
                },
                MetricRecord {
                    fold_id: "fold-1".into(),
                    variant: "subperiod".into(),
                    horizon_bars: 1,
                    output_name: protocol.output_name.clone(),
                    lens: FactorLens::Temporal,
                    metric: MetricId::Stability,
                    observation: observation(1.0),
                },
                MetricRecord {
                    fold_id: "fold-1".into(),
                    variant: "top-only".into(),
                    horizon_bars: 1,
                    output_name: protocol.output_name.clone(),
                    lens: FactorLens::Economic,
                    metric: MetricId::Economic,
                    observation: observation(0.1),
                },
            ],
            target_unavailable: Vec::new(),
            regime_evidence: Vec::new(),
            input_identities: vec!["dataset".into(), "protocol".into()],
            report_hash: String::new(),
        })
        .unwrap()
    }

    #[test]
    fn out_of_sample_policy_gates_enable_explicit_component_eligibility() {
        let user = Uuid::new_v4();
        let family_id = Uuid::new_v4();
        let trial_id = Uuid::new_v4();
        let candidate = candidate();
        let protocol = evaluation_protocol(user, family_id, trial_id, &candidate);
        let report = report(&protocol);
        let mut registry = crate::ResearchRegistry::default();
        registry
            .register_family(crate::ResearchFamilyDraft {
                family_id,
                user_id: user,
                root_candidate_hash: candidate.candidate_hash.clone(),
                parent_family_id: None,
                trials: vec![crate::ResearchTrialDraft {
                    trial_id,
                    candidate_hash: candidate.candidate_hash.clone(),
                    parameter_set_hash: "b".repeat(64),
                    target: crate::FactorTarget::FutureCloseReturn,
                    market_context: context(),
                    point_in_time_universe_id: "universe".into(),
                    observation_range: crate::ObservationRange {
                        start_time_ms: 0,
                        end_time_ms: 100,
                    },
                    evaluation_protocol_hash: protocol.protocol_hash.clone(),
                    derivation_hash: None,
                }],
            })
            .unwrap();
        let lineage = registry.lineage(user, trial_id).unwrap();
        let policy = PromotionPolicy::freeze(PromotionPolicyDraft {
            policy_id: Uuid::new_v4(),
            revision: 1,
            required_lenses: vec![FactorLens::Temporal, FactorLens::Economic],
            minimum_coverage: 0.8,
            minimum_samples: 30,
            maximum_holm_p_value: 0.05,
            require_subperiod_sign_consistency: true,
            require_cost_aware_economic: true,
        })
        .unwrap();
        let mut promotion_protocol = PromotionProtocol {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            protocol_id: Uuid::new_v4(),
            user_id: user,
            candidate_hash: candidate.candidate_hash.clone(),
            output_name: "value".into(),
            family_id,
            trial_id,
            lineage_trial_ids: lineage.trial_ids.clone(),
            lineage_hash: lineage.lineage_hash.clone(),
            report_hashes: vec![report.report_hash.clone()],
            policy_hash: policy.policy_hash.clone(),
            engine_identity: protocol.engine_identity.clone(),
            protocol_hash: String::new(),
        };
        promotion_protocol.protocol_hash = {
            let content = promotion_protocol.content();
            content_hash(&content).unwrap()
        };
        let dataset = crate::FactorDatasetManifest {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            dataset_id: "dataset".into(),
            protocol_hash: "c".repeat(64),
            candidate_hash: candidate.candidate_hash.clone(),
            scope: FactorScope::TimeSeries,
            feature_dataset_id: "feature".into(),
            feature_plan_hash: "a".repeat(64),
            market_data_snapshot_id: "snapshot".into(),
            point_in_time_universe_id: "universe".into(),
            market_context: context(),
            output_names: vec!["value".into()],
            observation_count: 100,
            payload_sha256: "d".repeat(64),
            engine_identity: protocol.engine_identity.clone(),
        };
        let eligibility = PromotionEligibility::check(PromotionEvidence {
            candidate: &candidate,
            dataset: &dataset,
            dataset_status: FactorDatasetStatus::Completed,
            evaluation_protocol: &protocol,
            reports: std::slice::from_ref(&report),
            policy: &policy,
            lineage: &lineage,
            promotion_protocol: &promotion_protocol,
            component: ComponentEligibilityEvidence {
                deterministic_execution: true,
                complete_source_provenance: true,
                abi_v2_expressible: true,
                buildable: true,
            },
        })
        .unwrap();
        assert!(eligibility.research_validated);
        assert!(eligibility.component_eligible);
        assert!(eligibility.gate(PromotionGate::CompleteSourceProvenance));

        let decision = FactorPromotionDecision::freeze(PromotionDecisionDraft {
            decision_id: Uuid::new_v4(),
            user_id: user,
            candidate_hash: candidate.candidate_hash.clone(),
            output_name: "value".into(),
            state: PromotionDecisionState::ResearchValidated,
            report_hashes: vec![report.report_hash.clone()],
            policy_hash: policy.policy_hash.clone(),
            evidence_state: EvaluationEvidenceState::OutOfSample,
            supersedes: None,
        })
        .unwrap();
        let mut ledger = PromotionDecisionLedger::default();
        ledger
            .append(decision, &promotion_protocol, &eligibility)
            .unwrap();
        assert!(
            ledger
                .m12_eligibility(M12EligibilityInput {
                    user_id: user,
                    dataset_status: FactorDatasetStatus::Completed,
                    dataset: &dataset,
                    evaluation_protocol: &protocol,
                    report: &report,
                    report_hash: report.report_hash.clone(),
                    policy: &policy,
                    promotion_protocol: &promotion_protocol,
                })
                .eligible
        );
    }

    #[test]
    fn custom_multi_output_requires_every_output_to_be_component_eligible() {
        let user = Uuid::new_v4();
        let candidate = custom_candidate();
        let mut ledger = PromotionDecisionLedger::default();
        for output in &candidate.outputs {
            let decision = FactorPromotionDecision::freeze(PromotionDecisionDraft {
                decision_id: Uuid::new_v4(),
                user_id: user,
                candidate_hash: candidate.candidate_hash.clone(),
                output_name: output.name.clone(),
                state: PromotionDecisionState::ComponentEligible,
                report_hashes: vec!["c".repeat(64)],
                policy_hash: "d".repeat(64),
                evidence_state: EvaluationEvidenceState::OutOfSample,
                supersedes: None,
            })
            .unwrap();
            ledger.records.insert(
                decision.decision_id,
                PromotionDecisionRecord {
                    decision,
                    promotion_protocol_hash: "e".repeat(64),
                    eligibility_gates: Vec::new(),
                    component: ComponentEligibilityEvidence::default(),
                },
            );
        }
        assert_eq!(
            ledger.m14_component_outputs(user, &candidate),
            vec!["first".to_owned(), "second".to_owned()]
        );
        let first_id = ledger.records.keys().next().copied().unwrap();
        ledger.records.remove(&first_id);
        assert!(ledger.m14_component_outputs(user, &candidate).is_empty());
    }

    #[test]
    fn policy_revision_changes_identity_and_template_has_explicit_gates() {
        let first =
            PromotionPolicy::conservative_template(Uuid::new_v4(), 1, FactorScope::TimeSeries)
                .unwrap();
        let second = PromotionPolicy::freeze(PromotionPolicyDraft {
            policy_id: first.policy_id,
            revision: 2,
            required_lenses: first.required_lenses.clone(),
            minimum_coverage: first.minimum_coverage,
            minimum_samples: first.minimum_samples,
            maximum_holm_p_value: first.maximum_holm_p_value,
            require_subperiod_sign_consistency: first.require_subperiod_sign_consistency,
            require_cost_aware_economic: first.require_cost_aware_economic,
        })
        .unwrap();
        assert_ne!(first.policy_hash, second.policy_hash);
        assert_eq!(
            first.required_lenses,
            vec![FactorLens::Temporal, FactorLens::Economic]
        );
    }

    #[test]
    fn rejected_decisions_are_append_only_and_library_is_current_projection() {
        let user = Uuid::new_v4();
        let candidate_hash = "a".repeat(64);
        let report_hash = "b".repeat(64);
        let policy_hash = "c".repeat(64);
        let trial_id = Uuid::new_v4();
        let engine = crate::ResearchEngineProvenance {
            engine_id: "native".into(),
            engine_version: "1".into(),
            adapter: "native".into(),
            target_triple: "test".into(),
            build_id: "build".into(),
            environment: BTreeMap::new(),
            parameters: BTreeMap::new(),
            input_identities: vec!["input".into()],
        };
        let protocol = PromotionProtocol {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            protocol_id: Uuid::new_v4(),
            user_id: user,
            candidate_hash: candidate_hash.clone(),
            output_name: "value".into(),
            family_id: Uuid::new_v4(),
            trial_id,
            lineage_trial_ids: vec![trial_id],
            lineage_hash: "d".repeat(64),
            report_hashes: vec![report_hash.clone()],
            policy_hash: policy_hash.clone(),
            engine_identity: engine,
            protocol_hash: String::new(),
        };
        let mut protocol = protocol;
        protocol.protocol_hash = {
            let content = protocol.content();
            content_hash(&content).unwrap()
        };
        protocol.validate().unwrap();
        let mut ledger = PromotionDecisionLedger::default();
        let eligibility = PromotionEligibility {
            candidate_hash: candidate_hash.clone(),
            output_name: "value".into(),
            promotion_protocol_hash: protocol.protocol_hash.clone(),
            gates: Vec::new(),
            research_validated: false,
            component_eligible: false,
            verified: true,
        };
        let decision = FactorPromotionDecision::freeze(PromotionDecisionDraft {
            decision_id: Uuid::new_v4(),
            user_id: user,
            candidate_hash: candidate_hash.clone(),
            output_name: "value".into(),
            state: PromotionDecisionState::Rejected,
            report_hashes: vec![report_hash.clone()],
            policy_hash: policy_hash.clone(),
            evidence_state: EvaluationEvidenceState::Unknown,
            supersedes: None,
        })
        .unwrap();
        ledger
            .append(decision.clone(), &protocol, &eligibility)
            .unwrap();

        let validated = FactorPromotionDecision::freeze(PromotionDecisionDraft {
            decision_id: Uuid::new_v4(),
            user_id: user,
            candidate_hash: candidate_hash.clone(),
            output_name: "value".into(),
            state: PromotionDecisionState::ResearchValidated,
            report_hashes: vec![report_hash],
            policy_hash,
            evidence_state: EvaluationEvidenceState::OutOfSample,
            supersedes: Some(decision.decision_id),
        })
        .unwrap();
        ledger
            .append(
                validated,
                &protocol,
                &PromotionEligibility {
                    candidate_hash,
                    output_name: "value".into(),
                    promotion_protocol_hash: protocol.protocol_hash.clone(),
                    gates: Vec::new(),
                    research_validated: true,
                    component_eligible: false,
                    verified: true,
                },
            )
            .unwrap();
        assert_eq!(ledger.decisions(user).len(), 2);
        assert_eq!(ledger.library(user).unwrap().entries.len(), 1);
        assert!(ledger.decisions(Uuid::new_v4()).is_empty());
    }
}
