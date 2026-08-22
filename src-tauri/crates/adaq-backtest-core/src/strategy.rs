use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrategyScope {
    SingleInstrument,
    Portfolio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvaluationWindow {
    Selection,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyWindow {
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

impl StrategyWindow {
    fn validate(&self, name: &str) -> Result<(), StrategyError> {
        if self.start_time_ms >= self.end_time_ms {
            return Err(StrategyError(format!("{name}-window-is-invalid")));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyBinding {
    pub slot: String,
    pub evidence_id: String,
    pub lineage_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyProject {
    pub strategy_id: String,
    pub user_id: String,
    pub revision: u64,
    pub strategy_archive_sha256: String,
    pub scope: StrategyScope,
    pub context_hash: String,
    pub context_start_time_ms: i64,
    pub context_end_time_ms: i64,
    pub selection_window: StrategyWindow,
    pub final_window: StrategyWindow,
    pub bindings: Vec<StrategyBinding>,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyError(pub String);

impl std::fmt::Display for StrategyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StrategyError {}

impl StrategyProject {
    pub fn create(
        strategy_id: impl Into<String>,
        user_id: impl Into<String>,
        strategy_archive_sha256: impl Into<String>,
        scope: StrategyScope,
        context_hash: impl Into<String>,
        context_start_time_ms: i64,
        context_end_time_ms: i64,
        selection_window: StrategyWindow,
        final_window: StrategyWindow,
        bindings: Vec<StrategyBinding>,
        parameters: BTreeMap<String, String>,
    ) -> Result<Self, StrategyError> {
        let project = Self {
            strategy_id: strategy_id.into(),
            user_id: user_id.into(),
            revision: 1,
            strategy_archive_sha256: strategy_archive_sha256.into(),
            scope,
            context_hash: context_hash.into(),
            context_start_time_ms,
            context_end_time_ms,
            selection_window,
            final_window,
            bindings,
            parameters,
        };
        project.validate()?;
        Ok(project)
    }

    pub fn revise(&self, mut revised: Self) -> Result<Self, StrategyError> {
        revised.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| StrategyError("strategy-revision-overflow".into()))?;
        revised.validate()?;
        Ok(revised)
    }

    pub fn validate(&self) -> Result<(), StrategyError> {
        if self.strategy_id.trim().is_empty()
            || self.user_id.trim().is_empty()
            || self.strategy_archive_sha256.trim().is_empty()
            || self.context_hash.trim().is_empty()
        {
            return Err(StrategyError(
                "strategy-project-identity-is-incomplete".into(),
            ));
        }
        if self.context_start_time_ms >= self.context_end_time_ms {
            return Err(StrategyError("strategy-context-window-is-invalid".into()));
        }
        self.selection_window.validate("selection")?;
        self.final_window.validate("final")?;
        if self.selection_window.end_time_ms > self.final_window.start_time_ms
            || self.selection_window.start_time_ms < self.context_start_time_ms
            || self.final_window.end_time_ms > self.context_end_time_ms
        {
            return Err(StrategyError(
                "strategy-evaluation-windows-overlap-or-escape-context".into(),
            ));
        }
        if self.bindings.iter().any(|binding| {
            binding.slot.trim().is_empty()
                || binding.evidence_id.trim().is_empty()
                || binding.lineage_hash.trim().is_empty()
        }) {
            return Err(StrategyError("strategy-binding-is-incomplete".into()));
        }
        if self
            .bindings
            .iter()
            .map(|binding| &binding.slot)
            .collect::<BTreeSet<_>>()
            .len()
            != self.bindings.len()
        {
            return Err(StrategyError(
                "strategy-binding-slots-are-not-unique".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrategyAttemptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyEvidence {
    pub attempt_id: String,
    pub project_revision: u64,
    pub context_hash: String,
    pub window: EvaluationWindow,
    pub run_ids: Vec<String>,
    pub provenance: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyAttempt {
    pub attempt_id: String,
    pub project_id: String,
    pub project_revision: u64,
    pub context_hash: String,
    pub window: EvaluationWindow,
    pub status: StrategyAttemptStatus,
    pub failure: Option<String>,
    pub evidence: Option<StrategyEvidence>,
}

impl StrategyAttempt {
    pub fn new(
        attempt_id: impl Into<String>,
        project: &StrategyProject,
        window: EvaluationWindow,
    ) -> Self {
        Self {
            attempt_id: attempt_id.into(),
            project_id: project.strategy_id.clone(),
            project_revision: project.revision,
            context_hash: project.context_hash.clone(),
            window,
            status: StrategyAttemptStatus::Pending,
            failure: None,
            evidence: None,
        }
    }

    pub fn begin(&mut self) -> Result<(), StrategyError> {
        if self.status != StrategyAttemptStatus::Pending {
            return Err(StrategyError("strategy-attempt-cannot-begin".into()));
        }
        self.status = StrategyAttemptStatus::Running;
        Ok(())
    }

    pub fn complete(
        &mut self,
        project: &StrategyProject,
        evidence: StrategyEvidence,
    ) -> Result<(), StrategyError> {
        if self.status != StrategyAttemptStatus::Running || self.evidence.is_some() {
            return Err(StrategyError(
                "strategy-evidence-cannot-be-overwritten".into(),
            ));
        }
        if self.project_id != project.strategy_id
            || self.project_revision != project.revision
            || self.context_hash != project.context_hash
            || evidence.attempt_id != self.attempt_id
            || evidence.project_revision != project.revision
            || evidence.context_hash != project.context_hash
            || evidence.window != self.window
        {
            return Err(StrategyError(
                "strategy-context-or-revision-mismatch".into(),
            ));
        }
        self.evidence = Some(evidence);
        self.status = StrategyAttemptStatus::Completed;
        Ok(())
    }

    pub fn fail(&mut self, reason: impl Into<String>) -> Result<(), StrategyError> {
        if !matches!(
            self.status,
            StrategyAttemptStatus::Pending | StrategyAttemptStatus::Running
        ) {
            return Err(StrategyError("strategy-attempt-cannot-fail".into()));
        }
        self.failure = Some(reason.into());
        self.status = StrategyAttemptStatus::Failed;
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), StrategyError> {
        if !matches!(
            self.status,
            StrategyAttemptStatus::Pending | StrategyAttemptStatus::Running
        ) {
            return Err(StrategyError("strategy-attempt-cannot-cancel".into()));
        }
        self.status = StrategyAttemptStatus::Cancelled;
        Ok(())
    }

    pub fn retry(&self, attempt_id: impl Into<String>) -> Result<Self, StrategyError> {
        if !matches!(
            self.status,
            StrategyAttemptStatus::Failed | StrategyAttemptStatus::Cancelled
        ) {
            return Err(StrategyError("strategy-attempt-cannot-retry".into()));
        }
        Ok(Self {
            attempt_id: attempt_id.into(),
            project_id: self.project_id.clone(),
            project_revision: self.project_revision,
            context_hash: self.context_hash.clone(),
            window: self.window,
            status: StrategyAttemptStatus::Pending,
            failure: None,
            evidence: None,
        })
    }

    pub fn recover_after_restart(&mut self) -> Result<(), StrategyError> {
        if self.status != StrategyAttemptStatus::Running {
            return Err(StrategyError("strategy-attempt-is-not-interrupted".into()));
        }
        self.status = StrategyAttemptStatus::Pending;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> StrategyProject {
        StrategyProject::create(
            "strategy-1",
            "alice",
            "a".repeat(64),
            StrategyScope::Portfolio,
            "context-1",
            0,
            10,
            StrategyWindow {
                start_time_ms: 1,
                end_time_ms: 4,
            },
            StrategyWindow {
                start_time_ms: 6,
                end_time_ms: 9,
            },
            vec![StrategyBinding {
                slot: "forecast".into(),
                evidence_id: "evidence-1".into(),
                lineage_hash: "lineage-1".into(),
            }],
            BTreeMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn revisions_and_windows_are_immutable_inputs_to_attempts() {
        let project = project();
        let revised = project
            .revise(StrategyProject {
                scope: StrategyScope::SingleInstrument,
                ..project.clone()
            })
            .unwrap();
        assert_eq!(revised.revision, 2);
        let attempt = StrategyAttempt::new("attempt-1", &project, EvaluationWindow::Selection);
        assert_eq!(attempt.project_revision, 1);
        assert!(
            StrategyProject::create(
                "strategy-1",
                "alice",
                "hash",
                StrategyScope::Portfolio,
                "context",
                0,
                10,
                StrategyWindow {
                    start_time_ms: 1,
                    end_time_ms: 8
                },
                StrategyWindow {
                    start_time_ms: 7,
                    end_time_ms: 9
                },
                vec![],
                BTreeMap::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn lifecycle_recovers_retries_and_rejects_mixed_context_evidence() {
        let project = project();
        let mut attempt = StrategyAttempt::new("attempt-1", &project, EvaluationWindow::Final);
        attempt.begin().unwrap();
        attempt.recover_after_restart().unwrap();
        attempt.begin().unwrap();
        let mut wrong = project.clone();
        wrong.context_hash = "other-context".into();
        assert!(
            attempt
                .complete(
                    &wrong,
                    StrategyEvidence {
                        attempt_id: "attempt-1".into(),
                        project_revision: 1,
                        context_hash: "other-context".into(),
                        window: EvaluationWindow::Final,
                        run_ids: vec!["run-1".into()],
                        provenance: BTreeMap::new(),
                    }
                )
                .is_err()
        );
        attempt.cancel().unwrap();
        let retry = attempt.retry("attempt-2").unwrap();
        assert_eq!(retry.status, StrategyAttemptStatus::Pending);
        assert!(retry.evidence.is_none());
    }

    #[test]
    fn completed_evidence_cannot_be_replaced() {
        let project = project();
        let mut attempt = StrategyAttempt::new("attempt-1", &project, EvaluationWindow::Final);
        attempt.begin().unwrap();
        let evidence = StrategyEvidence {
            attempt_id: "attempt-1".into(),
            project_revision: 1,
            context_hash: "context-1".into(),
            window: EvaluationWindow::Final,
            run_ids: vec!["run-1".into()],
            provenance: BTreeMap::new(),
        };
        attempt.complete(&project, evidence.clone()).unwrap();
        assert!(attempt.complete(&project, evidence).is_err());
    }
}
