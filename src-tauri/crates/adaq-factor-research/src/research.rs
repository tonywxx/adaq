use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ContractError, FactorMarketContext, FactorParameterValue, FactorTarget, GridSearch,
    MetricObservation, ObservationRange, ResearchFamily, ResearchTrial, ResearchTrialStatus,
    candidate::safe_diagnostic, canonical_json, content_hash, is_lower_kebab, is_sha256,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResearchError {
    Contract(ContractError),
    FamilyNotFound,
    TrialNotFound,
    Unauthorized,
    DuplicateIdentity,
    InvalidTransition,
    LineageOmission {
        expected: Vec<Uuid>,
        actual: Vec<Uuid>,
    },
}

impl std::fmt::Display for ResearchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::FamilyNotFound => formatter.write_str("research family was not found"),
            Self::TrialNotFound => formatter.write_str("research trial was not found"),
            Self::Unauthorized => formatter.write_str("research evidence is user-scoped"),
            Self::DuplicateIdentity => formatter.write_str("research identity already exists"),
            Self::InvalidTransition => formatter.write_str("invalid research trial transition"),
            Self::LineageOmission { .. } => {
                formatter.write_str("promotion protocol omits known related trials")
            }
        }
    }
}

impl std::error::Error for ResearchError {}

impl From<ContractError> for ResearchError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchTrialDraft {
    pub trial_id: Uuid,
    pub candidate_hash: String,
    pub parameter_set_hash: String,
    pub target: FactorTarget,
    pub market_context: FactorMarketContext,
    pub point_in_time_universe_id: String,
    pub observation_range: ObservationRange,
    pub evaluation_protocol_hash: String,
    pub derivation_hash: Option<String>,
}

impl ResearchTrialDraft {
    fn validate(&self, family_id: Uuid) -> Result<(), ContractError> {
        if self.trial_id.is_nil()
            || family_id.is_nil()
            || !is_sha256(&self.candidate_hash)
            || !is_sha256(&self.parameter_set_hash)
            || !is_sha256(&self.evaluation_protocol_hash)
            || self.point_in_time_universe_id.is_empty()
            || self.market_context.point_in_time_universe_id != self.point_in_time_universe_id
            || self
                .derivation_hash
                .as_deref()
                .is_some_and(|hash| !is_sha256(hash))
        {
            return Err(ContractError::Invalid(
                "Research Trial registration identity is invalid".into(),
            ));
        }
        self.market_context.validate()?;
        self.observation_range.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchFamilyDraft {
    pub family_id: Uuid,
    pub user_id: Uuid,
    pub root_candidate_hash: String,
    pub parent_family_id: Option<Uuid>,
    pub trials: Vec<ResearchTrialDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridSearchFamilyDraft {
    pub family_id: Uuid,
    pub user_id: Uuid,
    pub candidate_hash: String,
    pub parent_family_id: Option<Uuid>,
    pub plan: GridSearchPlan,
    pub target: FactorTarget,
    pub market_context: FactorMarketContext,
    pub point_in_time_universe_id: String,
    pub observation_range: ObservationRange,
    pub base_protocol_hash: String,
    pub derivation_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchTrialRegistration {
    pub trial_id: Uuid,
    pub family_id: Uuid,
    pub candidate_hash: String,
    pub parameter_set_hash: String,
    pub target: FactorTarget,
    pub market_context: FactorMarketContext,
    pub point_in_time_universe_id: String,
    pub observation_range: ObservationRange,
    pub evaluation_protocol_hash: String,
    pub derivation_hash: Option<String>,
    pub trial_hash: String,
}

impl ResearchTrialRegistration {
    fn from_draft(family_id: Uuid, draft: ResearchTrialDraft) -> Result<Self, ContractError> {
        draft.validate(family_id)?;
        let mut registration = Self {
            trial_id: draft.trial_id,
            family_id,
            candidate_hash: draft.candidate_hash,
            parameter_set_hash: draft.parameter_set_hash,
            target: draft.target,
            market_context: draft.market_context,
            point_in_time_universe_id: draft.point_in_time_universe_id,
            observation_range: draft.observation_range,
            evaluation_protocol_hash: draft.evaluation_protocol_hash,
            derivation_hash: draft.derivation_hash,
            trial_hash: String::new(),
        };
        registration.trial_hash = {
            let content = registration.content();
            content_hash(&content)?
        };
        registration.validate()?;
        Ok(registration)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.trial_id.is_nil()
            || self.family_id.is_nil()
            || !is_sha256(&self.candidate_hash)
            || !is_sha256(&self.parameter_set_hash)
            || !is_sha256(&self.evaluation_protocol_hash)
            || self.point_in_time_universe_id.is_empty()
            || self.market_context.point_in_time_universe_id != self.point_in_time_universe_id
            || !is_sha256(&self.trial_hash)
            || self
                .derivation_hash
                .as_deref()
                .is_some_and(|hash| !is_sha256(hash))
            || self.trial_hash != content_hash(&self.content())?
        {
            return Err(ContractError::Invalid(
                "Research Trial registration identity is invalid".into(),
            ));
        }
        self.market_context.validate()?;
        self.observation_range.validate()
    }

    fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            trial_id: Uuid,
            family_id: Uuid,
            candidate_hash: &'a str,
            parameter_set_hash: &'a str,
            target: FactorTarget,
            market_context: &'a FactorMarketContext,
            point_in_time_universe_id: &'a str,
            observation_range: &'a ObservationRange,
            evaluation_protocol_hash: &'a str,
            derivation_hash: &'a Option<String>,
        }
        Content {
            trial_id: self.trial_id,
            family_id: self.family_id,
            candidate_hash: &self.candidate_hash,
            parameter_set_hash: &self.parameter_set_hash,
            target: self.target,
            market_context: &self.market_context,
            point_in_time_universe_id: &self.point_in_time_universe_id,
            observation_range: &self.observation_range,
            evaluation_protocol_hash: &self.evaluation_protocol_hash,
            derivation_hash: &self.derivation_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchFamilyRegistration {
    pub family: ResearchFamily,
    pub trials: Vec<ResearchTrialRegistration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridSearchRegistration {
    pub family: ResearchFamilyRegistration,
    pub identities: Vec<GridSearchTrialIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineageDimension {
    Candidate,
    Target,
    Universe,
    Window,
    Derivation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchLineageRelation {
    pub trial_id: Uuid,
    pub family_id: Uuid,
    pub dimensions: Vec<LineageDimension>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchLineage {
    pub root_trial_id: Uuid,
    pub trial_ids: Vec<Uuid>,
    pub family_ids: Vec<Uuid>,
    pub relations: Vec<ResearchLineageRelation>,
    pub lineage_hash: String,
    #[serde(skip)]
    registry_derived: bool,
}

impl ResearchLineage {
    fn new(
        root_trial_id: Uuid,
        mut relations: Vec<ResearchLineageRelation>,
    ) -> Result<Self, ContractError> {
        relations.sort_by_key(|relation| relation.trial_id);
        let trial_ids = relations
            .iter()
            .map(|relation| relation.trial_id)
            .collect::<Vec<_>>();
        let family_ids = relations
            .iter()
            .map(|relation| relation.family_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let content = ResearchLineageContent {
            root_trial_id,
            trial_ids: &trial_ids,
            family_ids: &family_ids,
            relations: &relations,
        };
        let lineage_hash = content_hash(&content)?;
        let lineage = Self {
            root_trial_id,
            trial_ids,
            family_ids,
            relations,
            lineage_hash,
            registry_derived: true,
        };
        lineage.validate()?;
        Ok(lineage)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        let relation_trial_ids = self
            .relations
            .iter()
            .map(|relation| relation.trial_id)
            .collect::<Vec<_>>();
        let relation_family_ids = self
            .relations
            .iter()
            .map(|relation| relation.family_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if self.root_trial_id.is_nil()
            || self.trial_ids.is_empty()
            || self.trial_ids.windows(2).any(|pair| pair[0] >= pair[1])
            || !self.trial_ids.contains(&self.root_trial_id)
            || self.family_ids.is_empty()
            || self.family_ids.windows(2).any(|pair| pair[0] >= pair[1])
            || self.relations.len() != self.trial_ids.len()
            || self.trial_ids != relation_trial_ids
            || self.family_ids != relation_family_ids
            || self
                .relations
                .windows(2)
                .any(|pair| pair[0].trial_id >= pair[1].trial_id)
            || self.relations.iter().any(|relation| {
                relation.trial_id.is_nil()
                    || relation.family_id.is_nil()
                    || relation.dimensions.is_empty()
                    || relation
                        .dimensions
                        .iter()
                        .enumerate()
                        .any(|(index, dimension)| relation.dimensions[..index].contains(dimension))
            })
            || !is_sha256(&self.lineage_hash)
        {
            return Err(ContractError::Invalid("Research lineage is invalid".into()));
        }
        let content = ResearchLineageContent {
            root_trial_id: self.root_trial_id,
            trial_ids: &self.trial_ids,
            family_ids: &self.family_ids,
            relations: &self.relations,
        };
        if self.lineage_hash != content_hash(&content)? {
            return Err(ContractError::HashMismatch);
        }
        Ok(())
    }

    pub(crate) const fn is_registry_derived(&self) -> bool {
        self.registry_derived
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResearchLineageContent<'a> {
    root_trial_id: Uuid,
    trial_ids: &'a [Uuid],
    family_ids: &'a [Uuid],
    relations: &'a [ResearchLineageRelation],
}

#[derive(Debug, Clone, Default)]
pub struct ResearchRegistry {
    families: BTreeMap<Uuid, ResearchFamily>,
    registrations: BTreeMap<Uuid, ResearchTrialRegistration>,
    trials: BTreeMap<Uuid, ResearchTrial>,
}

impl ResearchRegistry {
    pub fn register_grid_search_family(
        &mut self,
        draft: GridSearchFamilyDraft,
    ) -> Result<GridSearchRegistration, ResearchError> {
        let identities = draft.plan.trial_identities(
            draft.family_id,
            &draft.candidate_hash,
            &draft.base_protocol_hash,
        )?;
        let trials = identities
            .iter()
            .map(|identity| ResearchTrialDraft {
                trial_id: identity.trial_id,
                candidate_hash: draft.candidate_hash.clone(),
                parameter_set_hash: identity.parameter_set_hash.clone(),
                target: draft.target,
                market_context: draft.market_context.clone(),
                point_in_time_universe_id: draft.point_in_time_universe_id.clone(),
                observation_range: draft.observation_range.clone(),
                evaluation_protocol_hash: identity.protocol_hash.clone(),
                derivation_hash: draft.derivation_hash.clone(),
            })
            .collect();
        let family = self.register_family(ResearchFamilyDraft {
            family_id: draft.family_id,
            user_id: draft.user_id,
            root_candidate_hash: draft.candidate_hash,
            parent_family_id: draft.parent_family_id,
            trials,
        })?;
        Ok(GridSearchRegistration { family, identities })
    }

    pub fn register_family(
        &mut self,
        draft: ResearchFamilyDraft,
    ) -> Result<ResearchFamilyRegistration, ResearchError> {
        if draft.family_id.is_nil()
            || draft.user_id.is_nil()
            || !is_sha256(&draft.root_candidate_hash)
            || draft.trials.is_empty()
            || self.families.contains_key(&draft.family_id)
        {
            return Err(if self.families.contains_key(&draft.family_id) {
                ResearchError::DuplicateIdentity
            } else {
                ContractError::Invalid("Research Family identity is invalid".into()).into()
            });
        }
        if let Some(parent_id) = draft.parent_family_id {
            let parent = self
                .families
                .get(&parent_id)
                .ok_or(ResearchError::FamilyNotFound)?;
            if parent.user_id != draft.user_id {
                return Err(ResearchError::Unauthorized);
            }
        }

        let mut seen = BTreeSet::new();
        let mut registrations = Vec::with_capacity(draft.trials.len());
        let mut trials = Vec::with_capacity(draft.trials.len());
        for trial_draft in draft.trials {
            if !seen.insert(trial_draft.trial_id)
                || self.registrations.contains_key(&trial_draft.trial_id)
            {
                return Err(ResearchError::DuplicateIdentity);
            }
            let registration = ResearchTrialRegistration::from_draft(draft.family_id, trial_draft)?;
            let trial = ResearchTrial {
                trial_id: registration.trial_id,
                family_id: registration.family_id,
                candidate_hash: registration.candidate_hash.clone(),
                protocol_hash: registration.evaluation_protocol_hash.clone(),
                status: ResearchTrialStatus::Registered,
                report_hash: None,
                raw_statistic: None,
                p_value: None,
                holm_adjusted: None,
                related_trial_ids: Vec::new(),
                diagnostic: None,
            };
            trial.validate()?;
            registrations.push(registration);
            trials.push(trial);
        }

        let mut family = ResearchFamily {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            family_id: draft.family_id,
            user_id: draft.user_id,
            root_candidate_hash: draft.root_candidate_hash,
            parent_family_id: draft.parent_family_id,
            registered_trial_ids: seen.iter().copied().collect(),
            lineage_hash: String::new(),
        };
        family.lineage_hash = {
            let content = family.content();
            content_hash(&content)?
        };
        family.validate()?;
        for registration in &registrations {
            registration.validate()?;
        }
        self.families.insert(family.family_id, family.clone());
        for registration in &registrations {
            self.registrations
                .insert(registration.trial_id, registration.clone());
        }
        for trial in &trials {
            self.trials.insert(trial.trial_id, trial.clone());
        }
        Ok(ResearchFamilyRegistration {
            family,
            trials: registrations,
        })
    }

    pub fn family(&self, user_id: Uuid, family_id: Uuid) -> Result<ResearchFamily, ResearchError> {
        let family = self
            .families
            .get(&family_id)
            .ok_or(ResearchError::FamilyNotFound)?;
        (family.user_id == user_id)
            .then(|| family.clone())
            .ok_or(ResearchError::Unauthorized)
    }

    pub fn trial(&self, user_id: Uuid, trial_id: Uuid) -> Result<ResearchTrial, ResearchError> {
        let trial = self
            .trials
            .get(&trial_id)
            .ok_or(ResearchError::TrialNotFound)?;
        let family = self
            .families
            .get(&trial.family_id)
            .ok_or(ResearchError::FamilyNotFound)?;
        (family.user_id == user_id)
            .then(|| trial.clone())
            .ok_or(ResearchError::Unauthorized)
    }

    pub fn registration(
        &self,
        user_id: Uuid,
        trial_id: Uuid,
    ) -> Result<ResearchTrialRegistration, ResearchError> {
        let registration = self
            .registrations
            .get(&trial_id)
            .ok_or(ResearchError::TrialNotFound)?;
        let family = self.family(user_id, registration.family_id)?;
        (family.family_id == registration.family_id)
            .then(|| registration.clone())
            .ok_or(ResearchError::Unauthorized)
    }

    pub fn record_trial(
        &mut self,
        user_id: Uuid,
        trial_id: Uuid,
        status: ResearchTrialStatus,
        report_hash: Option<String>,
        raw_statistic: Option<MetricObservation>,
        p_value: Option<MetricObservation>,
        diagnostic: Option<String>,
    ) -> Result<ResearchTrial, ResearchError> {
        if status == ResearchTrialStatus::Registered {
            return Err(ResearchError::InvalidTransition);
        }
        let mut trial = self.trial(user_id, trial_id)?;
        let diagnostic = diagnostic.map(|diagnostic| safe_diagnostic(&diagnostic));
        if !allowed_transition(trial.status, status) {
            return Err(ResearchError::InvalidTransition);
        }
        if report_hash.as_deref().is_some_and(|hash| !is_sha256(hash)) {
            return Err(
                ContractError::Invalid("Research Trial report hash is invalid".into()).into(),
            );
        }
        validate_probability(p_value.as_ref())?;
        if p_value.is_some() && raw_statistic.is_none() {
            return Err(ContractError::Invalid(
                "Research Trial p-values require a raw statistic".into(),
            )
            .into());
        }
        if trial.status == ResearchTrialStatus::Completed {
            if report_hash.is_some()
                || raw_statistic.is_some()
                || p_value.is_some()
                || diagnostic.is_some()
            {
                return Err(ResearchError::InvalidTransition);
            }
        } else {
            trial.report_hash = report_hash;
            trial.raw_statistic = raw_statistic;
            trial.p_value = p_value;
            trial.diagnostic = diagnostic;
        }
        trial.status = status;
        trial.validate()?;
        self.trials.insert(trial_id, trial.clone());
        Ok(trial)
    }

    pub fn lineage(&self, user_id: Uuid, trial_id: Uuid) -> Result<ResearchLineage, ResearchError> {
        let root = self.registration(user_id, trial_id)?;
        let mut relations = Vec::new();
        for registration in self.registrations.values() {
            let Some(family) = self.families.get(&registration.family_id) else {
                continue;
            };
            if family.user_id != user_id {
                continue;
            }
            let dimensions = matching_dimensions(&root, registration);
            if !dimensions.is_empty() {
                relations.push(ResearchLineageRelation {
                    trial_id: registration.trial_id,
                    family_id: family.family_id,
                    dimensions,
                });
            }
        }
        ResearchLineage::new(trial_id, relations).map_err(Into::into)
    }

    pub fn apply_holm_bonferroni(
        &mut self,
        user_id: Uuid,
        trial_id: Uuid,
    ) -> Result<HolmBonferroniCorrection, ResearchError> {
        let lineage = self.lineage(user_id, trial_id)?;
        let inputs = lineage
            .trial_ids
            .iter()
            .map(|id| {
                let trial = self.trials.get(id).ok_or(ResearchError::TrialNotFound)?;
                Ok((*id, trial.p_value.clone()))
            })
            .collect::<Result<Vec<_>, ResearchError>>()?;
        let correction = holm_bonferroni(&inputs)?;
        for (id, adjusted) in &correction.adjusted_p_values {
            let trial = self
                .trials
                .get_mut(id)
                .ok_or(ResearchError::TrialNotFound)?;
            trial.holm_adjusted = Some(adjusted.clone());
            trial.validate()?;
        }
        Ok(correction)
    }

    pub fn freeze_promotion_protocol(
        &self,
        draft: crate::PromotionProtocolDraft,
    ) -> Result<crate::PromotionProtocol, ResearchError> {
        let trial = self.trial(draft.user_id, draft.trial_id)?;
        let registration = self.registration(draft.user_id, draft.trial_id)?;
        if draft.family_id != registration.family_id
            || draft.candidate_hash != registration.candidate_hash
            || draft.output_name.is_empty()
        {
            return Err(
                ContractError::Invalid("Promotion Protocol target is invalid".into()).into(),
            );
        }
        let lineage = self.lineage(draft.user_id, draft.trial_id)?;
        let mut actual = draft.lineage_trial_ids.clone();
        actual.sort_unstable();
        actual.dedup();
        if actual != lineage.trial_ids {
            return Err(ResearchError::LineageOmission {
                expected: lineage.trial_ids,
                actual,
            });
        }
        if draft.report_hashes.is_empty()
            || draft
                .report_hashes
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || draft.report_hashes.iter().any(|hash| !is_sha256(hash))
        {
            return Err(
                ContractError::Invalid("Promotion Protocol reports are invalid".into()).into(),
            );
        }
        let known_reports = lineage
            .trial_ids
            .iter()
            .filter_map(|id| {
                self.trials
                    .get(id)
                    .and_then(|trial| trial.report_hash.as_deref())
            })
            .collect::<BTreeSet<_>>();
        if draft
            .report_hashes
            .iter()
            .any(|hash| !known_reports.contains(hash.as_str()))
            || trial
                .report_hash
                .as_deref()
                .is_none_or(|hash| !draft.report_hashes.iter().any(|reported| reported == hash))
        {
            return Err(ContractError::Invalid(
                "Promotion Protocol must cite registered Trial reports".into(),
            )
            .into());
        }
        crate::PromotionProtocol::freeze(draft, lineage.lineage_hash).map_err(Into::into)
    }
}

fn allowed_transition(from: ResearchTrialStatus, to: ResearchTrialStatus) -> bool {
    matches!(
        (from, to),
        (
            ResearchTrialStatus::Registered,
            ResearchTrialStatus::Completed
                | ResearchTrialStatus::Failed
                | ResearchTrialStatus::Cancelled
                | ResearchTrialStatus::Rejected
                | ResearchTrialStatus::Superseded
        ) | (
            ResearchTrialStatus::Completed,
            ResearchTrialStatus::Rejected | ResearchTrialStatus::Superseded
        )
    )
}

fn matching_dimensions(
    left: &ResearchTrialRegistration,
    right: &ResearchTrialRegistration,
) -> Vec<LineageDimension> {
    let mut dimensions = Vec::new();
    if left.candidate_hash == right.candidate_hash {
        dimensions.push(LineageDimension::Candidate);
    }
    if left.target == right.target {
        dimensions.push(LineageDimension::Target);
    }
    if left.point_in_time_universe_id == right.point_in_time_universe_id {
        dimensions.push(LineageDimension::Universe);
    }
    if left.observation_range == right.observation_range {
        dimensions.push(LineageDimension::Window);
    }
    if left.derivation_hash.is_some()
        && left.derivation_hash == right.derivation_hash
        && right.derivation_hash.is_some()
    {
        dimensions.push(LineageDimension::Derivation);
    }
    dimensions
}

fn validate_probability(observation: Option<&MetricObservation>) -> Result<(), ContractError> {
    let Some(observation) = observation else {
        return Ok(());
    };
    observation.validate()?;
    if observation
        .value()
        .is_some_and(|value| !(0.0..=1.0).contains(&value))
    {
        return Err(ContractError::Invalid(
            "p-values must be finite values between zero and one".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridSearchParameter {
    pub name: String,
    pub values: Vec<FactorParameterValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridSearchAssignment {
    pub name: String,
    pub value: FactorParameterValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridSearchCombination {
    pub index: u64,
    pub assignments: Vec<GridSearchAssignment>,
    pub parameter_set_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridSearchPlan {
    pub parameters: Vec<GridSearchParameter>,
    pub search: GridSearch,
    pub plan_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridSearchTrialIdentity {
    pub trial_id: Uuid,
    pub index: u64,
    pub parameter_set_hash: String,
    pub trial_hash: String,
    pub protocol_hash: String,
}

impl GridSearch {
    pub fn combinations(&self) -> Result<Vec<Vec<u64>>, ContractError> {
        self.validate()?;
        (0..self.trial_count)
            .map(|mut index| {
                let mut values = vec![0; self.parameter_cardinalities.len()];
                for position in (0..values.len()).rev() {
                    values[position] = index % self.parameter_cardinalities[position];
                    index /= self.parameter_cardinalities[position];
                }
                Ok(values)
            })
            .collect()
    }
}

impl GridSearchPlan {
    pub fn new(parameters: Vec<GridSearchParameter>) -> Result<Self, ContractError> {
        if parameters.is_empty() {
            return Err(ContractError::Invalid(
                "Grid Search requires a parameter".into(),
            ));
        }
        let mut names = BTreeSet::new();
        for parameter in &parameters {
            if !is_lower_kebab(&parameter.name)
                || !names.insert(parameter.name.as_str())
                || parameter.values.is_empty()
            {
                return Err(ContractError::Invalid(
                    "Grid Search parameters must be unique and non-empty".into(),
                ));
            }
            let mut serialized = BTreeSet::new();
            for value in &parameter.values {
                let bytes = canonical_json(value)?;
                if !serialized.insert(bytes) {
                    return Err(ContractError::Invalid(
                        "Grid Search parameter values must be unique".into(),
                    ));
                }
            }
        }
        let search = GridSearch::new(
            parameters
                .iter()
                .map(|parameter| parameter.values.len() as u64)
                .collect(),
        )?;
        let plan_hash = content_hash(&parameters)?;
        let plan = Self {
            parameters,
            search,
            plan_hash,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if !is_sha256(&self.plan_hash)
            || self.search.parameter_cardinalities
                != self
                    .parameters
                    .iter()
                    .map(|parameter| parameter.values.len() as u64)
                    .collect::<Vec<_>>()
            || self.plan_hash != content_hash(&self.parameters)?
        {
            return Err(ContractError::HashMismatch);
        }
        self.search.validate()
    }

    pub fn combinations(&self) -> Result<Vec<GridSearchCombination>, ContractError> {
        self.validate()?;
        self.search
            .combinations()?
            .into_iter()
            .enumerate()
            .map(|(index, positions)| {
                let assignments = self
                    .parameters
                    .iter()
                    .zip(positions)
                    .map(|(parameter, position)| GridSearchAssignment {
                        name: parameter.name.clone(),
                        value: parameter.values[position as usize].clone(),
                    })
                    .collect::<Vec<_>>();
                let parameter_set_hash = content_hash(&assignments)?;
                Ok(GridSearchCombination {
                    index: index as u64,
                    assignments,
                    parameter_set_hash,
                })
            })
            .collect()
    }

    pub fn trial_identities(
        &self,
        family_id: Uuid,
        candidate_hash: &str,
        base_protocol_hash: &str,
    ) -> Result<Vec<GridSearchTrialIdentity>, ContractError> {
        self.validate()?;
        if family_id.is_nil() || !is_sha256(candidate_hash) || !is_sha256(base_protocol_hash) {
            return Err(ContractError::Invalid(
                "Grid Search trial identity inputs are invalid".into(),
            ));
        }
        self.combinations()?
            .into_iter()
            .map(|combination| {
                let protocol_hash = content_hash(&GridProtocolIdentity {
                    plan_hash: &self.plan_hash,
                    base_protocol_hash,
                    index: combination.index,
                    parameter_set_hash: &combination.parameter_set_hash,
                })?;
                let trial_hash = content_hash(&GridTrialIdentity {
                    family_id,
                    candidate_hash,
                    protocol_hash: &protocol_hash,
                    parameter_set_hash: &combination.parameter_set_hash,
                })?;
                Ok(GridSearchTrialIdentity {
                    trial_id: trial_id_from_hash(&trial_hash)?,
                    index: combination.index,
                    parameter_set_hash: combination.parameter_set_hash,
                    trial_hash,
                    protocol_hash,
                })
            })
            .collect()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GridProtocolIdentity<'a> {
    plan_hash: &'a str,
    base_protocol_hash: &'a str,
    index: u64,
    parameter_set_hash: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GridTrialIdentity<'a> {
    family_id: Uuid,
    candidate_hash: &'a str,
    protocol_hash: &'a str,
    parameter_set_hash: &'a str,
}

fn trial_id_from_hash(hash: &str) -> Result<Uuid, ContractError> {
    if hash.len() != 64 || !is_sha256(hash) {
        return Err(ContractError::Invalid(
            "Grid Search Trial identity hash is invalid".into(),
        ));
    }
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hash[index * 2..index * 2 + 2], 16).map_err(|_| {
            ContractError::Invalid("Grid Search Trial identity hash is invalid".into())
        })?;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HolmBonferroniCorrection {
    pub family_size: usize,
    pub adjusted_p_values: BTreeMap<Uuid, MetricObservation>,
}

pub fn holm_bonferroni(
    trials: &[(Uuid, Option<MetricObservation>)],
) -> Result<HolmBonferroniCorrection, ContractError> {
    if trials.is_empty() {
        return Err(ContractError::Invalid(
            "Holm-Bonferroni requires a registered Trial family".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut available = Vec::new();
    for (trial_id, observation) in trials {
        if trial_id.is_nil() || !ids.insert(*trial_id) {
            return Err(ContractError::Invalid(
                "Holm-Bonferroni Trial identities must be unique".into(),
            ));
        }
        validate_probability(observation.as_ref())?;
        if let Some(observation) = observation
            && let Some(value) = observation.value()
        {
            available.push((*trial_id, value, observation.clone()));
        }
    }
    available.sort_by(|left, right| {
        left.1
            .total_cmp(&right.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let family_size = trials.len();
    let mut adjusted_p_values = trials
        .iter()
        .map(|(trial_id, _)| {
            (
                *trial_id,
                MetricObservation::available(1.0, 0).expect("finite Holm fallback"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut previous = 0.0;
    for (rank, (trial_id, p_value, original)) in available.iter().enumerate() {
        let adjusted = (p_value * (family_size - rank) as f64)
            .max(previous)
            .min(1.0);
        previous = adjusted;
        adjusted_p_values.insert(
            *trial_id,
            MetricObservation::available(adjusted, original_sample_count(original))?,
        );
    }
    Ok(HolmBonferroniCorrection {
        family_size,
        adjusted_p_values,
    })
}

fn original_sample_count(observation: &MetricObservation) -> u64 {
    match observation {
        MetricObservation::Available { sample_count, .. }
        | MetricObservation::Unavailable { sample_count, .. } => *sample_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FactorLens, PromotionPolicy, PromotionPolicyDraft};

    fn context(universe: &str) -> FactorMarketContext {
        FactorMarketContext {
            venue: "venue".into(),
            asset_class: "asset".into(),
            bar_interval: "1h".into(),
            price_basis: "close".into(),
            valuation_currency: "USD".into(),
            point_in_time_universe_id: universe.into(),
        }
    }

    fn trial(
        _family_id: Uuid,
        trial_id: Uuid,
        candidate: &str,
        universe: &str,
    ) -> ResearchTrialDraft {
        ResearchTrialDraft {
            trial_id,
            candidate_hash: candidate.repeat(64 / candidate.len()),
            parameter_set_hash: "b".repeat(64),
            target: FactorTarget::FutureCloseReturn,
            market_context: context(universe),
            point_in_time_universe_id: universe.into(),
            observation_range: ObservationRange {
                start_time_ms: 0,
                end_time_ms: 100,
            },
            evaluation_protocol_hash: "c".repeat(64),
            derivation_hash: None,
        }
    }

    #[test]
    fn grid_is_lexicographic_and_each_combination_has_independent_identity() {
        let plan = GridSearchPlan::new(vec![
            GridSearchParameter {
                name: "window".into(),
                values: vec![
                    FactorParameterValue::Integer(5),
                    FactorParameterValue::Integer(10),
                ],
            },
            GridSearchParameter {
                name: "decay".into(),
                values: vec![
                    FactorParameterValue::Integer(1),
                    FactorParameterValue::Integer(2),
                ],
            },
        ])
        .unwrap();
        let combinations = plan.combinations().unwrap();
        assert_eq!(
            combinations[0].assignments[0].value,
            FactorParameterValue::Integer(5)
        );
        assert_eq!(
            combinations[1].assignments[1].value,
            FactorParameterValue::Integer(2)
        );
        let identities = plan
            .trial_identities(Uuid::new_v4(), &"a".repeat(64), &"b".repeat(64))
            .unwrap();
        assert_eq!(identities.len(), 4);
        assert!(
            identities
                .iter()
                .all(|identity| !identity.trial_id.is_nil())
        );
        assert_ne!(identities[0].trial_hash, identities[1].trial_hash);
        assert_ne!(identities[0].protocol_hash, identities[1].protocol_hash);
    }

    #[test]
    fn grid_plan_rejects_more_than_256_cartesian_trials() {
        let values = |count: i64| {
            (0..count)
                .map(FactorParameterValue::Integer)
                .collect::<Vec<_>>()
        };
        assert!(
            GridSearchPlan::new(vec![
                GridSearchParameter {
                    name: "first".into(),
                    values: values(16),
                },
                GridSearchParameter {
                    name: "second".into(),
                    values: values(17),
                },
            ])
            .is_err()
        );
    }

    #[test]
    fn registered_grid_combinations_enter_family_wise_correction() {
        let user = Uuid::new_v4();
        let family_id = Uuid::new_v4();
        let plan = GridSearchPlan::new(vec![GridSearchParameter {
            name: "window".into(),
            values: vec![
                FactorParameterValue::Integer(5),
                FactorParameterValue::Integer(10),
            ],
        }])
        .unwrap();
        let mut registry = ResearchRegistry::default();
        let registration = registry
            .register_grid_search_family(GridSearchFamilyDraft {
                family_id,
                user_id: user,
                candidate_hash: "a".repeat(64),
                parent_family_id: None,
                plan,
                target: FactorTarget::FutureCloseReturn,
                market_context: context("universe"),
                point_in_time_universe_id: "universe".into(),
                observation_range: ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 100,
                },
                base_protocol_hash: "b".repeat(64),
                derivation_hash: None,
            })
            .unwrap();
        assert_eq!(registration.family.trials.len(), 2);
        assert_eq!(registration.identities.len(), 2);
        for (index, (trial, identity)) in registration
            .family
            .trials
            .iter()
            .zip(&registration.identities)
            .enumerate()
        {
            registry
                .record_trial(
                    user,
                    trial.trial_id,
                    ResearchTrialStatus::Completed,
                    Some(identity.trial_hash.clone()),
                    Some(MetricObservation::available(1.0, 10).unwrap()),
                    Some(MetricObservation::available(0.01 + index as f64 * 0.01, 10).unwrap()),
                    None,
                )
                .unwrap();
        }
        let correction = registry
            .apply_holm_bonferroni(user, registration.family.trials[0].trial_id)
            .unwrap();
        assert_eq!(correction.family_size, 2);
        assert_eq!(correction.adjusted_p_values.len(), 2);
        assert!(registration.family.trials.iter().all(|trial| {
            registry
                .trial(user, trial.trial_id)
                .unwrap()
                .holm_adjusted
                .is_some()
        }));
    }

    #[test]
    fn holm_counts_trials_without_statistics_as_non_significant() {
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let correction = holm_bonferroni(&[
            (
                ids[0],
                Some(MetricObservation::available(0.01, 10).unwrap()),
            ),
            (ids[1], None),
            (
                ids[2],
                Some(MetricObservation::available(0.02, 10).unwrap()),
            ),
        ])
        .unwrap();
        assert_eq!(correction.family_size, 3);
        assert_eq!(correction.adjusted_p_values[&ids[1]].value(), Some(1.0));
        assert_eq!(correction.adjusted_p_values[&ids[0]].value(), Some(0.03));
    }

    #[test]
    fn registry_retains_failures_and_rejects_incomplete_promotion_lineage() {
        let user = Uuid::new_v4();
        let family_id = Uuid::new_v4();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut registry = ResearchRegistry::default();
        registry
            .register_family(ResearchFamilyDraft {
                family_id,
                user_id: user,
                root_candidate_hash: "a".repeat(64),
                parent_family_id: None,
                trials: vec![
                    trial(family_id, first, "a", "universe"),
                    trial(family_id, second, "a", "universe"),
                ],
            })
            .unwrap();
        registry
            .record_trial(
                user,
                first,
                ResearchTrialStatus::Failed,
                None,
                None,
                None,
                Some("/Users/tony/private/secret diagnostic".into()),
            )
            .unwrap();
        assert_eq!(
            registry.trial(user, first).unwrap().status,
            ResearchTrialStatus::Failed
        );
        let safe_diagnostic = registry.trial(user, first).unwrap().diagnostic.unwrap();
        assert!(safe_diagnostic.contains("<private>"));
        assert!(!safe_diagnostic.contains("/Users/"));
        let lineage = registry.lineage(user, first).unwrap();
        assert_eq!(lineage.trial_ids.len(), 2);
        let policy = PromotionPolicy::freeze(PromotionPolicyDraft {
            policy_id: Uuid::new_v4(),
            revision: 1,
            required_lenses: vec![FactorLens::Temporal],
            minimum_coverage: 0.8,
            minimum_samples: 10,
            maximum_holm_p_value: 0.05,
            require_subperiod_sign_consistency: true,
            require_cost_aware_economic: true,
        })
        .unwrap();
        let error = registry
            .freeze_promotion_protocol(crate::PromotionProtocolDraft {
                protocol_id: Uuid::new_v4(),
                user_id: user,
                candidate_hash: "a".repeat(64),
                output_name: "momentum".into(),
                family_id,
                trial_id: second,
                lineage_trial_ids: vec![second],
                report_hashes: vec!["c".repeat(64)],
                policy_hash: policy.policy_hash,
                engine_identity: crate::ResearchEngineProvenance {
                    engine_id: "native".into(),
                    engine_version: "1".into(),
                    adapter: "native".into(),
                    target_triple: "test".into(),
                    build_id: "build".into(),
                    environment: BTreeMap::new(),
                    parameters: BTreeMap::new(),
                    input_identities: vec!["input".into()],
                },
            })
            .unwrap_err();
        assert!(matches!(error, ResearchError::LineageOmission { .. }));
    }

    #[test]
    fn lineage_spans_same_user_families_but_preserves_user_isolation() {
        let user = Uuid::new_v4();
        let other_user = Uuid::new_v4();
        let family_one = Uuid::new_v4();
        let family_two = Uuid::new_v4();
        let family_three = Uuid::new_v4();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let other = Uuid::new_v4();
        let mut registry = ResearchRegistry::default();
        let mut root_trial = trial(family_one, first, "a", "universe");
        root_trial.derivation_hash = Some("e".repeat(64));
        registry
            .register_family(ResearchFamilyDraft {
                family_id: family_one,
                user_id: user,
                root_candidate_hash: "a".repeat(64),
                parent_family_id: None,
                trials: vec![root_trial],
            })
            .unwrap();
        for (family_id, user_id, trial_id) in [
            (family_two, user, second),
            (family_three, other_user, other),
        ] {
            registry
                .register_family(ResearchFamilyDraft {
                    family_id,
                    user_id,
                    root_candidate_hash: "a".repeat(64),
                    parent_family_id: None,
                    trials: vec![trial(family_id, trial_id, "a", "universe")],
                })
                .unwrap();
        }
        let derived = Uuid::new_v4();
        let mut derived_trial = trial(family_one, derived, "d", "other-universe");
        derived_trial.observation_range = ObservationRange {
            start_time_ms: 200,
            end_time_ms: 300,
        };
        derived_trial.derivation_hash = Some("e".repeat(64));
        registry
            .register_family(ResearchFamilyDraft {
                family_id: Uuid::new_v4(),
                user_id: user,
                root_candidate_hash: "d".repeat(64),
                parent_family_id: None,
                trials: vec![derived_trial],
            })
            .unwrap();
        let lineage = registry.lineage(user, first).unwrap();
        assert_eq!(lineage.trial_ids.len(), 3);
        assert!(lineage.trial_ids.contains(&second));
        assert!(lineage.trial_ids.contains(&derived));
        assert!(lineage.relations.iter().any(|relation| {
            relation.trial_id == derived
                && relation.dimensions.contains(&LineageDimension::Derivation)
        }));
        assert!(!lineage.trial_ids.contains(&other));
        assert!(matches!(
            registry.trial(other_user, first),
            Err(ResearchError::Unauthorized)
        ));
    }

    #[test]
    fn completed_trial_evidence_is_not_overwritten_by_later_status() {
        let user = Uuid::new_v4();
        let family_id = Uuid::new_v4();
        let trial_id = Uuid::new_v4();
        let report_hash = "d".repeat(64);
        let mut registry = ResearchRegistry::default();
        registry
            .register_family(ResearchFamilyDraft {
                family_id,
                user_id: user,
                root_candidate_hash: "a".repeat(64),
                parent_family_id: None,
                trials: vec![trial(family_id, trial_id, "a", "universe")],
            })
            .unwrap();
        registry
            .record_trial(
                user,
                trial_id,
                ResearchTrialStatus::Completed,
                Some(report_hash.clone()),
                Some(MetricObservation::available(1.0, 10).unwrap()),
                Some(MetricObservation::available(0.01, 10).unwrap()),
                Some("retained".into()),
            )
            .unwrap();
        registry.apply_holm_bonferroni(user, trial_id).unwrap();
        registry
            .record_trial(
                user,
                trial_id,
                ResearchTrialStatus::Superseded,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let retained = registry.trial(user, trial_id).unwrap();
        assert_eq!(retained.report_hash.as_deref(), Some(report_hash.as_str()));
        assert_eq!(retained.diagnostic.as_deref(), Some("retained"));
        assert!(retained.is_significant_at(0.05));
    }
}
