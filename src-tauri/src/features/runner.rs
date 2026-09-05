//! Feature execution bodies and the adapter for the shared Research Queue.
//!
//! The queue executes one heavy Attempt at a time in admission order. Pending
//! Attempts live in SQLite and survive restarts; stale
//! Running Attempts recover at open time. Cancellation reaches the
//! evaluation loops between observations, and a Running Attempt is only
//! terminalized after its worker has stopped and released its evidence.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use adaq_backtest_core::MarketDataSnapshot;
use adaq_data_core::market::PriceBasis;
use adaq_data_core::{BarInterval, OhlcvBar, next_bar_open_time_ms};
use adaq_feature_engine::{
    FeatureEngine, FeatureEvaluationError, FeatureEvaluationInput, FeatureInputEvent,
    FeatureMarketBar, FeatureObservation, FeaturePlan, FittedTransformationArtifact,
    MaterializationAttempt, ObservationRange, PointInTimeInstrumentUniverse,
    TransformationFittingProtocol, UniverseEvidenceState,
};

use super::{
    ActiveAttempt, FeaturesInner, FittingAttemptRecord, UserFeatureResetBlock, bounded_diagnostic,
    instrument_id_for, store, string,
};
use crate::{
    forecast_signal_dataset::hash,
    research_queue::{
        QueueAdmission, QueueRunResult, QueueTicket, ResearchQueue, ResearchQueueAdapter, WorkKind,
    },
    user::validate_user,
};

static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(0);
const PROGRESS_FLUSH_EVENTS: usize = 256;

pub(super) struct FeatureQueueAdapter {
    inner: Weak<FeaturesInner>,
    kind: WorkKind,
}

impl FeatureQueueAdapter {
    pub(super) fn new(inner: Arc<FeaturesInner>, kind: WorkKind) -> Self {
        Self {
            inner: Arc::downgrade(&inner),
            kind,
        }
    }
}

impl ResearchQueueAdapter for FeatureQueueAdapter {
    fn pending_attempts(&self) -> Result<Vec<QueueAdmission>, String> {
        let Some(inner) = self.inner.upgrade() else {
            return Ok(Vec::new());
        };
        match self.kind {
            WorkKind::FeatureFitting => {
                let database = inner.source.database()?;
                Ok(store::FeatureStore::new(&database)
                    .pending_fitting_attempts()?
                    .into_iter()
                    .map(|attempt| QueueAdmission {
                        user_id: attempt.user_id,
                        attempt_id: attempt.attempt_id,
                    })
                    .collect())
            }
            WorkKind::FeatureMaterialization => inner
                .materialization
                .pending_attempts()
                .map_err(string)
                .map(|attempts| {
                    attempts
                        .into_iter()
                        .map(|attempt| QueueAdmission {
                            user_id: attempt.user_id,
                            attempt_id: attempt.attempt_id,
                        })
                        .collect()
                }),
            _ => Ok(Vec::new()),
        }
    }

    fn execute(&self, ticket: QueueTicket) -> QueueRunResult {
        let Some(inner) = self.inner.upgrade() else {
            return QueueRunResult::Stale;
        };
        match self.kind {
            WorkKind::FeatureFitting => {
                let attempt = match inner.source.database() {
                    Ok(database) => match store::FeatureStore::new(&database)
                        .pending_fitting_attempt(&ticket.user_id, &ticket.attempt_id)
                    {
                        Ok(attempt) => attempt,
                        Err(error) => return QueueRunResult::Retryable(error),
                    },
                    Err(error) => return QueueRunResult::Retryable(error),
                };
                let Some((record, protocol_json)) = attempt else {
                    return QueueRunResult::Stale;
                };
                run_fitting(&inner, &record, &protocol_json);
                QueueRunResult::Consumed
            }
            WorkKind::FeatureMaterialization => {
                let attempt = match inner
                    .materialization
                    .attempt(&ticket.user_id, &ticket.attempt_id)
                {
                    Ok(attempt) => attempt,
                    Err(adaq_feature_engine::MaterializationStoreError::AttemptNotFound) => {
                        return QueueRunResult::Stale;
                    }
                    Err(error) => return QueueRunResult::Retryable(error.to_string()),
                };
                if attempt.status != adaq_feature_engine::MaterializationAttemptStatus::Pending {
                    return QueueRunResult::Stale;
                }
                run_materialization(&inner, &attempt);
                QueueRunResult::Consumed
            }
            _ => QueueRunResult::Stale,
        }
    }

    fn request_shutdown(&self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        if let Ok(attempts) = inner.attempts.lock() {
            for attempt in attempts.values() {
                attempt.cancelled.store(true, Ordering::Relaxed);
            }
        }
    }
}

pub(super) fn new_attempt_id(seed: &str) -> String {
    let nonce = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hash(format!("{seed}:{now}:{nonce}").as_bytes())
}

#[cfg(test)]
pub(super) fn attempt_started(inner: &FeaturesInner, kind: &str, attempt_id: &str) {
    if let Ok(hook) = inner.attempt_started_hook.lock()
        && let Some(hook) = hook.as_ref()
    {
        hook(kind, attempt_id);
    }
}

#[cfg(not(test))]
pub(super) fn attempt_started(_inner: &FeaturesInner, _kind: &str, _attempt_id: &str) {}

/// One heavy Attempt's final state is only written after its worker has
/// exited the evaluation loop and released its evidence.
enum Outcome {
    Completed,
    Superseded,
    Cancelled,
    Failed { code: String, diagnostic: String },
}

fn run_materialization(inner: &FeaturesInner, attempt: &MaterializationAttempt) {
    let attempt_id = attempt.attempt_id.clone();
    let user_id = attempt.user_id.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    if let Ok(mut attempts) = inner.attempts.lock() {
        attempts.insert(
            attempt_id.clone(),
            ActiveAttempt {
                user_id: user_id.clone(),
                cancelled: cancelled.clone(),
            },
        );
    }
    let outcome = run_materialization_body(inner, &user_id, &attempt_id, &cancelled);
    match outcome {
        Outcome::Completed | Outcome::Superseded => {}
        Outcome::Cancelled => {
            if let Err(error) = inner.materialization.cancel(&user_id, &attempt_id) {
                eprintln!("Feature Materialization cancellation finalization failed: {error}");
            }
        }
        Outcome::Failed { code, diagnostic } => {
            if let Err(error) = inner.materialization.fail_with_diagnostic(
                &user_id,
                &attempt_id,
                &code,
                &diagnostic,
            ) {
                eprintln!("Feature Materialization failure finalization failed: {error}");
            }
        }
    }
    if let Ok(mut attempts) = inner.attempts.lock() {
        attempts.remove(&attempt_id);
    }
}

fn run_materialization_body(
    inner: &FeaturesInner,
    user_id: &str,
    attempt_id: &str,
    cancelled: &AtomicBool,
) -> Outcome {
    let store = &inner.materialization;
    if store.begin(user_id, attempt_id).is_err() {
        return Outcome::Superseded;
    }
    attempt_started(inner, "materialization", attempt_id);
    if cancelled.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    let (request, plan) = match store.execution_evidence(user_id, attempt_id) {
        Ok(evidence) => evidence,
        Err(error) => {
            return store_outcome(error);
        }
    };
    let artifacts = match load_plan_artifacts(inner, user_id, &plan) {
        Ok(artifacts) => artifacts,
        Err(outcome) => return outcome,
    };
    let events = if super::plan_has_cross_sectional_scope(&plan) {
        match cross_sectional_events(
            inner,
            user_id,
            Some(&request.snapshot_id),
            &request.point_in_time_universe_id,
            &request.observation_range,
            &request.valuation_currency,
        ) {
            Ok(events) => events,
            Err(error) => {
                return Outcome::Failed {
                    code: "invalid-feature-evidence".into(),
                    diagnostic: bounded_diagnostic(error),
                };
            }
        }
    } else if request.materialize_universe_members {
        match time_series_universe_events(
            inner,
            user_id,
            &request.snapshot_id,
            &request.point_in_time_universe_id,
            &request.observation_range,
        ) {
            Ok(events) => events,
            Err(error) => {
                return Outcome::Failed {
                    code: "invalid-feature-evidence".into(),
                    diagnostic: bounded_diagnostic(error),
                };
            }
        }
    } else {
        let (snapshot, bars) = match inner
            .source
            .snapshot_for_user(user_id, &request.snapshot_id)
        {
            Ok(evidence) => evidence,
            Err(error) => {
                return Outcome::Failed {
                    code: "feature-evidence-not-found".into(),
                    diagnostic: bounded_diagnostic(error),
                };
            }
        };
        match snapshot_events(&snapshot, &bars, &request.observation_range) {
            Ok(events) => events,
            Err(error) => {
                return Outcome::Failed {
                    code: "invalid-feature-evidence".into(),
                    diagnostic: bounded_diagnostic(error),
                };
            }
        }
    };
    let output_count: usize = plan
        .definitions()
        .iter()
        .map(|definition| definition.outputs().len())
        .sum::<usize>()
        .max(1);
    let total = u64::try_from(
        events
            .len()
            .saturating_mul(output_count)
            .saturating_mul(event_member_count(&events)),
    )
    .unwrap_or(u64::MAX);
    if let Err(error) = store.record_progress(user_id, attempt_id, 0, total) {
        return store_outcome(error);
    }
    let engine = FeatureEngine::new(plan.engine_identity());
    let mut evaluator = match engine.evaluator_with_artifacts(plan.clone(), &artifacts) {
        Ok(evaluator) => evaluator,
        Err(error) => return evaluation_outcome(&error),
    };
    let mut observations: Vec<FeatureObservation> = Vec::new();
    let mut completed: u64 = 0;
    for (index, event) in events.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Outcome::Cancelled;
        }
        match evaluator.observe(event.clone()) {
            Ok(batch) => {
                completed = completed.saturating_add(batch.len() as u64);
                observations.extend(batch);
            }
            Err(error) => return evaluation_outcome(&error),
        }
        if index % PROGRESS_FLUSH_EVENTS == PROGRESS_FLUSH_EVENTS - 1 || index + 1 == events.len() {
            let _ = store.record_progress(user_id, attempt_id, completed, total);
        }
    }
    if cancelled.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    let names: Vec<String> = plan
        .definitions()
        .iter()
        .flat_map(|definition| {
            definition
                .outputs()
                .iter()
                .map(|output| output.name.clone())
        })
        .collect();
    let name_refs = names.iter().map(String::as_str).collect::<Vec<_>>();
    if let Err(error) = store.stage(user_id, attempt_id, &name_refs, &observations) {
        return store_outcome(error);
    }
    drop(observations);
    // Cancellation wins before atomic publication begins; publication wins
    // after it begins. The staging file is cleaned by cancellation.
    if cancelled.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    match store.publish(user_id, attempt_id) {
        Ok(_) => Outcome::Completed,
        Err(error) => store_outcome(error),
    }
}

fn run_fitting(inner: &FeaturesInner, record: &FittingAttemptRecord, protocol_json: &str) {
    let attempt_id = record.attempt_id.clone();
    let user_id = record.user_id.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    if let Ok(mut attempts) = inner.attempts.lock() {
        attempts.insert(
            attempt_id.clone(),
            ActiveAttempt {
                user_id: user_id.clone(),
                cancelled: cancelled.clone(),
            },
        );
    }
    let outcome = run_fitting_body(inner, record, protocol_json, &cancelled);
    let database = inner.source.database();
    match outcome {
        Outcome::Completed | Outcome::Superseded => {}
        Outcome::Cancelled => {
            if let Ok(database) = database
                && let Err(error) = store::FeatureStore::new(&database).cancel_fitting(
                    &user_id,
                    &attempt_id,
                    &["pending", "running"],
                )
            {
                eprintln!("Feature Fitting cancellation finalization failed: {error}");
            }
        }
        Outcome::Failed { code, diagnostic } => {
            if let Ok(database) = database
                && let Err(error) = store::FeatureStore::new(&database).fail_fitting(
                    &user_id,
                    &attempt_id,
                    &code,
                    &diagnostic,
                )
            {
                eprintln!("Feature Fitting failure finalization failed: {error}");
            }
        }
    }
    if let Ok(mut attempts) = inner.attempts.lock() {
        attempts.remove(&attempt_id);
    }
}

fn run_fitting_body(
    inner: &FeaturesInner,
    record: &FittingAttemptRecord,
    protocol_json: &str,
    cancelled: &AtomicBool,
) -> Outcome {
    let user_id = &record.user_id;
    let attempt_id = &record.attempt_id;
    let started = {
        let database = match inner.source.database() {
            Ok(database) => database,
            Err(error) => return failure("feature-database-unavailable", error),
        };
        match store::FeatureStore::new(&database).mark_fitting_running(user_id, attempt_id) {
            Ok(started) => started,
            Err(error) => return failure("feature-database-unavailable", error),
        }
    };
    if !started {
        return Outcome::Superseded;
    }
    attempt_started(inner, "fitting", attempt_id);
    if cancelled.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    let identity = match adaq_feature_engine::FeatureEngineIdentity::native() {
        Ok(identity) => identity,
        Err(error) => return failure("unsupported-fitting-engine-identity", error.to_string()),
    };
    let protocol =
        match TransformationFittingProtocol::load_for_engine(protocol_json.as_bytes(), &identity) {
            Ok(protocol) => protocol,
            Err(error) => return failure(error.code(), error.to_string()),
        };
    let plan = match FeaturePlan::load_for_engine(record.plan_json.as_bytes(), &identity) {
        Ok(plan) => plan,
        Err(error) => return failure(error.code(), error.to_string()),
    };
    let artifacts = match load_plan_artifacts(inner, user_id, &plan) {
        Ok(artifacts) => artifacts,
        Err(outcome) => return outcome,
    };
    let events = if super::plan_has_cross_sectional_scope(&plan) {
        match cross_sectional_events(
            inner,
            user_id,
            Some(protocol.snapshot_id()),
            protocol.point_in_time_universe_id(),
            protocol.fitting_window(),
            protocol.valuation_currency(),
        ) {
            Ok(events) => events,
            Err(error) => return failure("invalid-feature-evidence", error),
        }
    } else {
        let (snapshot, bars) = match inner
            .source
            .snapshot_for_user(user_id, protocol.snapshot_id())
        {
            Ok(evidence) => evidence,
            Err(error) => return failure("feature-evidence-not-found", error),
        };
        match snapshot_events(&snapshot, &bars, protocol.fitting_window()) {
            Ok(events) => events,
            Err(error) => return failure("invalid-feature-evidence", error),
        }
    };
    let output_count: usize = plan
        .definitions()
        .iter()
        .map(|definition| definition.outputs().len())
        .sum::<usize>()
        .max(1);
    let total = i64::try_from(
        events
            .len()
            .saturating_mul(output_count)
            .saturating_mul(event_member_count(&events)),
    )
    .unwrap_or(i64::MAX);
    set_fitting_progress(inner, user_id, attempt_id, 0, total);
    let engine = FeatureEngine::new(plan.engine_identity());
    let mut evaluator = match engine.evaluator_with_artifacts(plan.clone(), &artifacts) {
        Ok(evaluator) => evaluator,
        Err(error) => return evaluation_outcome(&error),
    };
    let mut observations: Vec<FeatureObservation> = Vec::new();
    let mut completed: i64 = 0;
    for (index, event) in events.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Outcome::Cancelled;
        }
        match evaluator.observe(event.clone()) {
            Ok(batch) => {
                completed = completed.saturating_add(batch.len() as i64);
                observations.extend(batch);
            }
            Err(error) => return evaluation_outcome(&error),
        }
        if index % PROGRESS_FLUSH_EVENTS == PROGRESS_FLUSH_EVENTS - 1 || index + 1 == events.len() {
            set_fitting_progress(inner, user_id, attempt_id, completed, total);
        }
    }
    if cancelled.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    let artifact = match protocol.fit(&observations, store::now_ms()) {
        Ok(artifact) => artifact,
        Err(error) => {
            let diagnostic = serde_json::json!({
                "code": error.code(),
                "instrumentId": error.instrument_id,
                "eligibleSamples": error.eligible_samples,
                "minimumSamples": error.minimum_samples,
            });
            return failure(error.code(), diagnostic.to_string());
        }
    };
    drop(observations);
    let artifact_json = match String::from_utf8(artifact.to_json()) {
        Ok(json) => json,
        Err(error) => return failure("invalid-fitted-artifact", error.to_string()),
    };
    let database = match inner.source.database() {
        Ok(database) => database,
        Err(error) => return failure("feature-database-unavailable", error),
    };
    let store = store::FeatureStore::new(&database);
    if let Err(error) = store.publish_artifact(
        user_id,
        artifact.artifact_id(),
        protocol.protocol_hash(),
        &artifact_json,
    ) {
        return failure("fitted-artifact-publication-failed", error);
    }
    match store.complete_fitting(user_id, attempt_id, artifact.artifact_id()) {
        Ok(true) => Outcome::Completed,
        Ok(false) => Outcome::Superseded,
        Err(error) => failure("fitted-artifact-publication-failed", error),
    }
}

fn set_fitting_progress(
    inner: &FeaturesInner,
    user_id: &str,
    attempt_id: &str,
    completed: i64,
    total: i64,
) {
    if let Ok(database) = inner.source.database() {
        let _ = store::FeatureStore::new(&database)
            .set_fitting_progress(user_id, attempt_id, completed, total);
    }
}

fn load_plan_artifacts(
    inner: &FeaturesInner,
    user_id: &str,
    plan: &FeaturePlan,
) -> Result<Vec<FittedTransformationArtifact>, Outcome> {
    let mut artifacts = Vec::new();
    if plan.artifacts().is_empty() {
        return Ok(artifacts);
    }
    let database = inner
        .source
        .database()
        .map_err(|error| failure("feature-database-unavailable", error))?;
    let store = store::FeatureStore::new(&database);
    for binding in plan.artifacts() {
        let record = store
            .artifact_for_user(user_id, &binding.artifact_id)
            .map_err(|error| failure("fitted-artifact-not-found", error))?;
        let artifact = FittedTransformationArtifact::load_for_engine(
            record.artifact_json.as_bytes(),
            &plan.engine_identity(),
        )
        .map_err(|error| failure(error.code(), error.to_string()))?;
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

/// Converts one immutable Snapshot into causal Feature input events: one
/// Observation per Closed Bar inside the range, with Bar Gap resets at the
/// recorded gaps. Observation Time and Available At both use the bar close
/// instant.
pub(super) fn snapshot_events(
    snapshot: &MarketDataSnapshot,
    bars: &[OhlcvBar],
    range: &ObservationRange,
) -> Result<Vec<FeatureInputEvent>, String> {
    let instrument_id = instrument_id_for(&snapshot.src, &snapshot.code);
    let mut events = Vec::new();
    let mut last_observation_time = i64::MIN;
    let mut gap_index = 0usize;
    for bar in bars {
        while gap_index < snapshot.gaps.len()
            && snapshot.gaps[gap_index].start_time_ms <= bar.open_time_ms
        {
            let start_time_ms = snapshot.gaps[gap_index].start_time_ms;
            gap_index += 1;
            if start_time_ms >= range.start_time_ms && start_time_ms < range.end_time_ms {
                // Observation Times are strictly causal: a Bar Gap recorded
                // at the previous bar's close instant moves one instant
                // forward instead of colliding with it.
                let gap_time = start_time_ms.max(last_observation_time + 1);
                events.push(FeatureInputEvent::bar_gap(
                    &instrument_id,
                    gap_time,
                    gap_time,
                ));
                last_observation_time = gap_time;
            }
        }
        let close = bar_close_time(snapshot.interval, bar.open_time_ms)?;
        if close < range.start_time_ms || close >= range.end_time_ms {
            continue;
        }
        events.push(FeatureInputEvent::observation(FeatureEvaluationInput::new(
            &instrument_id,
            close,
            close,
            FeatureMarketBar::from_ohlcv(bar.clone()),
        )));
        last_observation_time = close;
    }
    Ok(events)
}

/// Expands a time-series Plan over each immutable PIT Universe member. The
/// selected Snapshot remains the anchor identity and must be a member.
fn time_series_universe_events(
    inner: &FeaturesInner,
    user_id: &str,
    snapshot_id: &str,
    universe_id: &str,
    range: &ObservationRange,
) -> Result<Vec<FeatureInputEvent>, String> {
    let universe = inner
        .source
        .universe_snapshot_for_user(user_id, universe_id)?;
    if !universe
        .components
        .iter()
        .any(|component| component.snapshot_id == snapshot_id)
    {
        return Err("feature-snapshot-universe-identity-mismatch".into());
    }
    let mut components = universe.components.iter().collect::<Vec<_>>();
    components.sort_by_key(|component| {
        instrument_id_for(
            &component.dataset.instrument.venue.id,
            &component.dataset.instrument.code,
        )
    });
    let mut events = Vec::new();
    for component in components {
        let (snapshot, bars) = inner
            .source
            .snapshot_for_user(user_id, &component.snapshot_id)?;
        events.extend(snapshot_events(&snapshot, &bars, range)?);
    }
    Ok(events)
}

/// Builds one complete, deterministic batch per observation time from the
/// accepted Point-in-Time Universe and its immutable component Snapshots.
pub(super) fn cross_sectional_events(
    inner: &FeaturesInner,
    user_id: &str,
    snapshot_id: Option<&str>,
    universe_id: &str,
    range: &ObservationRange,
    valuation_currency: &str,
) -> Result<Vec<FeatureInputEvent>, String> {
    let universe = inner
        .source
        .universe_snapshot_for_user(user_id, universe_id)?;
    if let Some(snapshot_id) = snapshot_id.filter(|id| !id.trim().is_empty()) {
        if !universe
            .components
            .iter()
            .any(|component| component.snapshot_id == snapshot_id)
        {
            return Err("feature-snapshot-universe-identity-mismatch".into());
        }
        inner.source.snapshot_for_user(user_id, snapshot_id)?;
    }
    let market_context = adaq_feature_engine::FeatureMarketContext::new(
        universe.venue.clone(),
        universe.venue.kind,
        universe.interval,
        PriceBasis::Unadjusted,
        valuation_currency,
    )
    .map_err(|error| error.to_string())?;
    let mut members = universe
        .universe
        .instruments
        .iter()
        .map(|instrument| format!("{}:{}", instrument.venue.id, instrument.code))
        .collect::<Vec<_>>();
    members.sort_unstable();
    let evidence_state = match universe.universe.evidence_state.as_str() {
        "observed" => UniverseEvidenceState::Observed,
        "reconstructed" => UniverseEvidenceState::Reconstructed,
        _ => UniverseEvidenceState::Unknown,
    };
    let mut bars_by_instrument: BTreeMap<String, BTreeMap<i64, FeatureMarketBar>> = BTreeMap::new();
    for component in &universe.components {
        let (_, bars) = inner
            .source
            .snapshot_for_user(user_id, &component.snapshot_id)?;
        let instrument_id = format!(
            "{}:{}",
            component.dataset.instrument.venue.id, component.dataset.instrument.code
        );
        if !members.iter().any(|member| member == &instrument_id) {
            return Err("feature-universe-component-membership-mismatch".into());
        }
        let by_time = bars_by_instrument.entry(instrument_id).or_default();
        for mut bar in bars {
            let close = bar_close_time(universe.interval, bar.open_time_ms)?;
            if close < range.start_time_ms || close >= range.end_time_ms {
                continue;
            }
            // The engine validates Cross-Sectional bars at the batch's
            // Observation Time, which is the closed-bar instant here.
            bar.open_time_ms = close;
            by_time.insert(close, FeatureMarketBar::from_ohlcv(bar));
        }
    }
    let mut observation_times = bars_by_instrument
        .values()
        .flat_map(|by_time| by_time.keys().copied())
        .collect::<Vec<_>>();
    observation_times.sort_unstable();
    observation_times.dedup();

    observation_times
        .into_iter()
        .map(|time| {
            let market_context = market_context.clone();
            let pit_universe = PointInTimeInstrumentUniverse::new(
                universe.snapshot_id.clone(),
                time,
                members.clone(),
                market_context,
                evidence_state,
            )
            .map_err(|error| error.to_string())?;
            let inputs = members
                .iter()
                .map(|member| {
                    bars_by_instrument
                        .get(member)
                        .and_then(|by_time| by_time.get(&time))
                        .map(|bar| FeatureEvaluationInput::new(member, time, time, bar.clone()))
                        .unwrap_or_else(|| FeatureEvaluationInput::missing(member, time, time))
                })
                .collect();
            Ok(FeatureInputEvent::cross_sectional_batch(
                time,
                pit_universe,
                inputs,
            ))
        })
        .collect()
}

fn event_member_count(events: &[FeatureInputEvent]) -> usize {
    events
        .first()
        .map(|event| match event {
            FeatureInputEvent::CrossSectionalBatch(batch) => batch.universe.members.len(),
            _ => 1,
        })
        .unwrap_or(1)
}

fn bar_close_time(interval: BarInterval, open_time_ms: i64) -> Result<i64, String> {
    next_bar_open_time_ms(open_time_ms, interval).map_err(string)
}

fn evaluation_outcome(error: &FeatureEvaluationError) -> Outcome {
    // Typed errors preserve code/stage/node/Instrument/Observation Time and
    // safe diagnostics; localized summaries remain a frontend concern.
    let diagnostic = serde_json::to_string(error).unwrap_or_else(|_| error.code().to_owned());
    Outcome::Failed {
        code: error.code().into(),
        diagnostic: bounded_diagnostic(diagnostic),
    }
}

fn store_outcome(error: adaq_feature_engine::MaterializationStoreError) -> Outcome {
    Outcome::Failed {
        code: error.code().into(),
        diagnostic: bounded_diagnostic(error.to_string()),
    }
}

fn failure(code: impl Into<String>, diagnostic: impl Into<String>) -> Outcome {
    Outcome::Failed {
        code: code.into(),
        diagnostic: bounded_diagnostic(diagnostic.into()),
    }
}

/// Blocks new Start/Retry for one User, cancels that User's Pending and
/// Running Attempts, and waits for the runner to release them without
/// holding the SQLite mutex. The start gate is held while the block is
/// installed and Pending Attempts are cancelled, so no Start that passed
/// its block check earlier can insert an Attempt after the barrier.
pub(super) fn stop_all_for_user<'a>(
    inner: &'a FeaturesInner,
    queue: &ResearchQueue,
    user_id: &str,
) -> Result<UserFeatureResetBlock<'a>, String> {
    validate_user(user_id)?;
    let start_gate = inner.start_gate.lock().map_err(string)?;
    inner
        .reset_blocks
        .lock()
        .map_err(string)?
        .insert(user_id.to_string());
    let block = UserFeatureResetBlock {
        inner,
        user_id: user_id.to_string(),
    };
    {
        let database = inner.source.database()?;
        let store = store::FeatureStore::new(&database);
        for attempt_id in store.active_fitting_ids_for_user(user_id)? {
            let _ = store.cancel_fitting(user_id, &attempt_id, &["pending"]);
        }
    }
    for attempt in inner.materialization.attempts(user_id).map_err(string)? {
        if attempt.status == adaq_feature_engine::MaterializationAttemptStatus::Pending {
            let _ = inner.materialization.cancel(user_id, &attempt.attempt_id);
        }
    }
    let active_ids: Vec<String> = {
        let attempts = inner.attempts.lock().map_err(string)?;
        attempts
            .iter()
            .filter(|(_, active)| active.user_id == user_id)
            .map(|(attempt_id, _)| attempt_id.clone())
            .collect()
    };
    for attempt_id in &active_ids {
        if let Some(active) = inner.attempts.lock().map_err(string)?.get(attempt_id) {
            active.cancelled.store(true, Ordering::Relaxed);
        }
    }
    // Wake the worker so Pending Attempts it holds are finalized promptly.
    queue.wake();
    drop(start_gate);
    let timeout = *inner.reset_wait_timeout.lock().map_err(string)?;
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = {
            let attempts = inner.attempts.lock().map_err(string)?;
            attempts
                .values()
                .filter(|active| active.user_id == user_id)
                .count()
        };
        if remaining == 0 {
            break;
        }
        if Instant::now() >= deadline {
            return Err("Reset All could not stop Feature work within the allowed time".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(block)
}
