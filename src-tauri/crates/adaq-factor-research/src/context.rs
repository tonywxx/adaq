//! Host-owned, User-scoped research evidence handoff.
//!
//! This module contains the transport-independent contract. Persistence and
//! authorization remain Host responsibilities; clients receive projections.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{ContractError, content_hash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResearchStage {
    Features,
    Factors,
    Models,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceGrade {
    ProviderGraded,
    Degraded,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceAdmissionPolicy {
    pub allow_degraded: bool,
    pub allow_unknown: bool,
}

impl Default for EvidenceAdmissionPolicy {
    fn default() -> Self {
        Self {
            allow_degraded: false,
            allow_unknown: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceBinding {
    pub id: String,
    pub lineage_hash: String,
    pub user_id: String,
    pub market: String,
    pub venue: String,
    pub snapshot_id: String,
    pub universe_id: Option<String>,
    pub feature_id: Option<String>,
    pub factor_id: Option<String>,
    pub model_id: Option<String>,
    pub grade: EvidenceGrade,
    pub accessible: bool,
    pub complete: bool,
    pub fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchEvidenceContextDraft {
    pub user_id: String,
    pub market: String,
    pub venue: String,
    pub range_start_ms: i64,
    pub range_end_ms: i64,
    pub snapshot_id: String,
    pub universe_id: Option<String>,
    pub evidence: Vec<EvidenceBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchEvidenceContext {
    pub revision: u64,
    pub context_hash: String,
    pub draft: ResearchEvidenceContextDraft,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenResearchEvidence {
    pub operation_id: String,
    pub context_revision: u64,
    pub context_hash: String,
    pub stage: ResearchStage,
    pub snapshot_id: String,
    pub universe_id: Option<String>,
    pub lineage_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchEvidenceProjection {
    pub context_revision: u64,
    pub context_hash: String,
    pub market: String,
    pub venue: String,
    pub range_start_ms: i64,
    pub range_end_ms: i64,
    pub snapshot_id: String,
    pub universe_id: Option<String>,
    pub evidence: Vec<EvidenceBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextBlocker {
    InvalidRange,
    MissingSnapshot,
    MissingUniverse,
    InaccessibleEvidence(String),
    MixedMarketVenue { id: String },
    UncoveredEvidence(String),
    StaleEvidence(String),
    UnknownEvidence(String),
    DegradedEvidence(String),
    UserIsolation(String),
    InvalidLineage(String),
}

impl std::fmt::Display for ContextBlocker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRange => f.write_str("context time range must be increasing"),
            Self::MissingSnapshot => f.write_str("context requires an exact market data snapshot"),
            Self::MissingUniverse => f.write_str("this stage requires a point-in-time universe"),
            Self::InaccessibleEvidence(id) => write!(f, "evidence {id} is inaccessible"),
            Self::MixedMarketVenue { id } => write!(f, "evidence {id} has a mixed market or venue"),
            Self::UncoveredEvidence(id) => {
                write!(f, "evidence {id} does not cover the context range")
            }
            Self::StaleEvidence(id) => write!(f, "evidence {id} is stale"),
            Self::UnknownEvidence(id) => write!(f, "evidence {id} has unknown quality"),
            Self::DegradedEvidence(id) => write!(f, "evidence {id} is degraded for this stage"),
            Self::UserIsolation(id) => write!(f, "evidence {id} belongs to another user"),
            Self::InvalidLineage(id) => write!(f, "evidence {id} has invalid immutable lineage"),
        }
    }
}

impl std::error::Error for ContextBlocker {}

impl ResearchEvidenceContext {
    pub const SCHEMA_VERSION: &'static str = "1.0.0";

    pub fn establish(draft: ResearchEvidenceContextDraft) -> Result<Self, ContextBlocker> {
        Self::establish_for_stage(
            draft,
            ResearchStage::Features,
            EvidenceAdmissionPolicy::default(),
        )
    }

    pub fn establish_for_stage(
        draft: ResearchEvidenceContextDraft,
        stage: ResearchStage,
        policy: EvidenceAdmissionPolicy,
    ) -> Result<Self, ContextBlocker> {
        validate_draft(&draft, stage, policy)?;
        Self::from_draft(1, draft)
    }

    pub fn revise(&self, draft: ResearchEvidenceContextDraft) -> Result<Self, ContextBlocker> {
        self.revise_for_stage(
            draft,
            ResearchStage::Features,
            EvidenceAdmissionPolicy::default(),
        )
    }

    pub fn revise_for_stage(
        &self,
        draft: ResearchEvidenceContextDraft,
        stage: ResearchStage,
        policy: EvidenceAdmissionPolicy,
    ) -> Result<Self, ContextBlocker> {
        validate_draft(&draft, stage, policy)?;
        Self::from_draft(
            self.revision
                .checked_add(1)
                .ok_or(ContextBlocker::InvalidLineage("revision overflow".into()))?,
            draft,
        )
    }

    pub fn freeze(
        &self,
        operation_id: impl Into<String>,
        stage: ResearchStage,
    ) -> Result<FrozenResearchEvidence, ContextBlocker> {
        self.freeze_with_policy(operation_id, stage, EvidenceAdmissionPolicy::default())
    }

    pub fn freeze_with_policy(
        &self,
        operation_id: impl Into<String>,
        stage: ResearchStage,
        policy: EvidenceAdmissionPolicy,
    ) -> Result<FrozenResearchEvidence, ContextBlocker> {
        validate_draft(&self.draft, stage, policy)?;
        let mut lineage_hashes = self
            .draft
            .evidence
            .iter()
            .map(|e| e.lineage_hash.clone())
            .collect::<Vec<_>>();
        lineage_hashes.sort();
        Ok(FrozenResearchEvidence {
            operation_id: operation_id.into(),
            context_revision: self.revision,
            context_hash: self.context_hash.clone(),
            stage,
            snapshot_id: self.draft.snapshot_id.clone(),
            universe_id: self.draft.universe_id.clone(),
            lineage_hashes,
        })
    }

    pub fn revalidate(&self, current: &Self, stage: ResearchStage) -> Result<(), ContextBlocker> {
        if self.revision != current.revision || self.context_hash != current.context_hash {
            return Err(ContextBlocker::StaleEvidence(
                "context revision changed".into(),
            ));
        }
        self.revalidate_with_policy(current, stage, EvidenceAdmissionPolicy::default())
    }

    pub fn revalidate_with_policy(
        &self,
        current: &Self,
        stage: ResearchStage,
        policy: EvidenceAdmissionPolicy,
    ) -> Result<(), ContextBlocker> {
        if self.revision != current.revision || self.context_hash != current.context_hash {
            return Err(ContextBlocker::StaleEvidence(
                "context revision changed".into(),
            ));
        }
        validate_draft(&current.draft, stage, policy)
    }

    pub fn projection(&self) -> ResearchEvidenceProjection {
        ResearchEvidenceProjection {
            context_revision: self.revision,
            context_hash: self.context_hash.clone(),
            market: self.draft.market.clone(),
            venue: self.draft.venue.clone(),
            range_start_ms: self.draft.range_start_ms,
            range_end_ms: self.draft.range_end_ms,
            snapshot_id: self.draft.snapshot_id.clone(),
            universe_id: self.draft.universe_id.clone(),
            evidence: self.draft.evidence.clone(),
        }
    }

    fn from_draft(
        revision: u64,
        draft: ResearchEvidenceContextDraft,
    ) -> Result<Self, ContextBlocker> {
        let context_hash = content_hash(&draft)
            .map_err(|error| ContextBlocker::InvalidLineage(error.to_string()))?;
        Ok(Self {
            revision,
            context_hash,
            draft,
        })
    }
}

fn validate_draft(
    draft: &ResearchEvidenceContextDraft,
    stage: ResearchStage,
    policy: EvidenceAdmissionPolicy,
) -> Result<(), ContextBlocker> {
    if draft.user_id.trim().is_empty()
        || draft.market.trim().is_empty()
        || draft.venue.trim().is_empty()
        || draft.range_start_ms >= draft.range_end_ms
    {
        return Err(ContextBlocker::InvalidRange);
    }
    if draft.snapshot_id.trim().is_empty() {
        return Err(ContextBlocker::MissingSnapshot);
    }
    if matches!(stage, ResearchStage::Factors | ResearchStage::Models)
        && draft.universe_id.is_none()
    {
        return Err(ContextBlocker::MissingUniverse);
    }
    let mut lineage = BTreeSet::new();
    for evidence in &draft.evidence {
        if evidence.user_id != draft.user_id {
            return Err(ContextBlocker::UserIsolation(evidence.id.clone()));
        }
        if evidence.market != draft.market || evidence.venue != draft.venue {
            return Err(ContextBlocker::MixedMarketVenue {
                id: evidence.id.clone(),
            });
        }
        if !evidence.accessible {
            return Err(ContextBlocker::InaccessibleEvidence(evidence.id.clone()));
        }
        if !evidence.complete {
            return Err(ContextBlocker::UncoveredEvidence(evidence.id.clone()));
        }
        if !evidence.fresh {
            return Err(ContextBlocker::StaleEvidence(evidence.id.clone()));
        }
        match evidence.grade {
            EvidenceGrade::Unknown if !policy.allow_unknown => {
                return Err(ContextBlocker::UnknownEvidence(evidence.id.clone()));
            }
            EvidenceGrade::Degraded if !policy.allow_degraded => {
                return Err(ContextBlocker::DegradedEvidence(evidence.id.clone()));
            }
            _ => {}
        }
        if evidence.lineage_hash.is_empty() || !lineage.insert(&evidence.lineage_hash) {
            return Err(ContextBlocker::InvalidLineage(evidence.id.clone()));
        }
    }
    Ok(())
}

impl From<ContextBlocker> for ContractError {
    fn from(value: ContextBlocker) -> Self {
        Self::Invalid(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(user_id: &str) -> EvidenceBinding {
        EvidenceBinding {
            id: "snapshot-1".into(),
            lineage_hash: "lineage-1".into(),
            user_id: user_id.into(),
            market: "crypto".into(),
            venue: "okx".into(),
            snapshot_id: "snapshot-1".into(),
            universe_id: Some("universe-1".into()),
            feature_id: None,
            factor_id: None,
            model_id: None,
            grade: EvidenceGrade::ProviderGraded,
            accessible: true,
            complete: true,
            fresh: true,
        }
    }
    fn draft(user_id: &str) -> ResearchEvidenceContextDraft {
        ResearchEvidenceContextDraft {
            user_id: user_id.into(),
            market: "crypto".into(),
            venue: "okx".into(),
            range_start_ms: 1,
            range_end_ms: 2,
            snapshot_id: "snapshot-1".into(),
            universe_id: Some("universe-1".into()),
            evidence: vec![evidence(user_id)],
        }
    }

    #[test]
    fn establishes_and_projects_typed_context() {
        let context = ResearchEvidenceContext::establish(draft("alice")).unwrap();
        assert_eq!(context.projection().snapshot_id, "snapshot-1");
    }
    #[test]
    fn freezes_exact_lineage_and_revision() {
        let context = ResearchEvidenceContext::establish(draft("alice")).unwrap();
        let frozen = context
            .freeze("operation-1", ResearchStage::Factors)
            .unwrap();
        assert_eq!(frozen.context_revision, 1);
        assert_eq!(frozen.lineage_hashes, vec!["lineage-1"]);
    }
    #[test]
    fn rejects_mixed_market() {
        let mut value = draft("alice");
        value.evidence[0].market = "equity".into();
        assert!(matches!(
            ResearchEvidenceContext::establish(value),
            Err(ContextBlocker::MixedMarketVenue { .. })
        ));
    }
    #[test]
    fn rejects_cross_user_evidence() {
        assert!(matches!(
            ResearchEvidenceContext::establish(draft("bob")),
            Ok(_)
        ));
        let mut value = draft("alice");
        value.evidence[0].user_id = "bob".into();
        assert!(matches!(
            ResearchEvidenceContext::establish(value),
            Err(ContextBlocker::UserIsolation(_))
        ));
    }
    #[test]
    fn rejects_unknown_and_stale_evidence() {
        let mut value = draft("alice");
        value.evidence[0].grade = EvidenceGrade::Unknown;
        assert!(matches!(
            ResearchEvidenceContext::establish(value),
            Err(ContextBlocker::UnknownEvidence(_))
        ));
        let mut value = draft("alice");
        value.evidence[0].fresh = false;
        assert!(matches!(
            ResearchEvidenceContext::establish(value),
            Err(ContextBlocker::StaleEvidence(_))
        ));
    }
    #[test]
    fn revision_does_not_rebind_old_operation() {
        let context = ResearchEvidenceContext::establish(draft("alice")).unwrap();
        let mut changed = draft("alice");
        changed.snapshot_id = "snapshot-2".into();
        let revised = context.revise(changed).unwrap();
        assert!(
            context
                .revalidate(&revised, ResearchStage::Features)
                .is_err()
        );
    }
    #[test]
    fn factors_require_universe() {
        let mut value = draft("alice");
        value.universe_id = None;
        assert!(matches!(
            ResearchEvidenceContext::establish(value.clone())
                .unwrap()
                .freeze("op", ResearchStage::Factors),
            Err(ContextBlocker::MissingUniverse)
        ));
    }
}
