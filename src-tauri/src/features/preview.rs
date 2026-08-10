//! Bounded transient Draft Preview over immutable Snapshot evidence.
//!
//! Preview uses the production Feature Engine, never fits, creates no
//! evidence identity, and persists nothing. Cross-Sectional work always
//! observes complete Point-in-Time Universe batches; Pointwise and
//! Time-Series previews may be bounded by Observation Time selection.

use std::collections::BTreeMap;

use adaq_data_core::market::PriceBasis;
use adaq_feature_engine::{
    FeatureDefinition, FeatureEngine, FeatureEvaluationInput, FeatureInputEvent, FeatureMarketBar,
    FeatureObservation, FeaturePlan, FeaturePlanDraft, FeatureScope, FittedArtifactBinding,
    FittedTransformationArtifact, ObservationRange, PointInTimeInstrumentUniverse,
    UniverseEvidenceState,
};

use super::{
    FeaturePreviewRequest, FeaturePreviewView, FeaturesInner, MAX_PREVIEW_CROSS_SECTIONAL_BATCHES,
    MAX_PREVIEW_OBSERVATIONS, runner, store, string,
};

pub(super) fn preview(
    inner: &FeaturesInner,
    request: FeaturePreviewRequest,
) -> Result<FeaturePreviewView, String> {
    let identity = super::native_identity()?;
    let definition = FeatureDefinition::freeze(request.draft.clone())
        .map_err(|error| format!("definition-validation-failed: {:?}", error.codes()))?;
    let (bindings, artifacts) = load_preview_artifacts(inner, &request, &identity)?;
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition.clone()],
        artifacts: bindings,
        engine_identity: identity,
        ..FeaturePlanDraft::default()
    })
    .map_err(|error| format!("feature-plan-validation-failed: {:?}", error.codes()))?;
    let engine = FeatureEngine::new(plan.engine_identity());
    let mut evaluator = engine
        .evaluator_with_artifacts(plan, &artifacts)
        .map_err(|error| error.to_string())?;
    if definition.scope() == FeatureScope::CrossSectional {
        preview_cross_sectional(inner, &request, &mut evaluator)
    } else {
        preview_single_instrument(inner, &request, &mut evaluator)
    }
}

/// Preview only ever applies Artifacts that already exist for the User.
fn load_preview_artifacts(
    inner: &FeaturesInner,
    request: &FeaturePreviewRequest,
    identity: &adaq_feature_engine::FeatureEngineIdentity,
) -> Result<
    (
        Vec<FittedArtifactBinding>,
        Vec<FittedTransformationArtifact>,
    ),
    String,
> {
    let mut bindings = Vec::new();
    let mut artifacts = Vec::new();
    if request.artifact_ids.is_empty() {
        return Ok((bindings, artifacts));
    }
    let database = inner.source.database()?;
    let store = store::FeatureStore::new(&database);
    for artifact_id in &request.artifact_ids {
        let record = store.artifact_for_user(&request.user_id, artifact_id)?;
        let artifact = FittedTransformationArtifact::load_for_engine(
            record.artifact_json.as_bytes(),
            identity,
        )
        .map_err(|error| error.to_string())?;
        bindings.push(FittedArtifactBinding {
            artifact_id: artifact.artifact_id.clone(),
            eligible_at_ms: artifact.eligible_at_ms,
            fitted_output: artifact.fitted_output.clone(),
        });
        artifacts.push(artifact);
    }
    Ok((bindings, artifacts))
}

fn preview_single_instrument(
    inner: &FeaturesInner,
    request: &FeaturePreviewRequest,
    evaluator: &mut adaq_feature_engine::FeatureEvaluator,
) -> Result<FeaturePreviewView, String> {
    let snapshot_id = request
        .snapshot_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or("Feature Preview requires a Snapshot")?;
    let (snapshot, bars) = inner
        .source
        .snapshot_for_user(&request.user_id, snapshot_id)?;
    let range = ObservationRange {
        start_time_ms: request.start_time_ms.unwrap_or(i64::MIN),
        end_time_ms: request.end_time_ms.unwrap_or(i64::MAX),
    };
    let mut events = runner::snapshot_events(&snapshot, &bars, &range)?;
    let max_events = request
        .max_events
        .unwrap_or(MAX_PREVIEW_OBSERVATIONS)
        .min(MAX_PREVIEW_OBSERVATIONS);
    let truncated = events.len() > max_events;
    events.truncate(max_events);
    let observations = evaluate_all(evaluator, &events)?;
    Ok(FeaturePreviewView {
        observations,
        event_count: events.len(),
        truncated,
    })
}

/// Cross-Sectional Preview retains the complete Universe for every observed
/// batch; only the number of Observation Times is bounded.
fn preview_cross_sectional(
    inner: &FeaturesInner,
    request: &FeaturePreviewRequest,
    evaluator: &mut adaq_feature_engine::FeatureEvaluator,
) -> Result<FeaturePreviewView, String> {
    let universe_id = request
        .universe_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or("Cross-Sectional Feature Preview requires a Point-in-Time Universe")?;
    let valuation_currency = request
        .valuation_currency
        .as_deref()
        .filter(|currency| !currency.trim().is_empty())
        .ok_or("Cross-Sectional Feature Preview requires a valuation currency")?;
    let universe = inner
        .source
        .universe_snapshot_for_user(&request.user_id, universe_id)?;
    let market_context = adaq_feature_engine::FeatureMarketContext::new(
        universe.venue.clone(),
        universe.venue.kind,
        universe.interval,
        PriceBasis::Unadjusted,
        valuation_currency,
    )
    .map_err(|error| error.to_string())?;
    let members = universe
        .universe
        .instruments
        .iter()
        .map(|instrument| format!("{}:{}", instrument.venue.id, instrument.code))
        .collect::<Vec<_>>();
    let evidence_state = match universe.universe.evidence_state.as_str() {
        "observed" => UniverseEvidenceState::Observed,
        "reconstructed" => UniverseEvidenceState::Reconstructed,
        _ => UniverseEvidenceState::Unknown,
    };
    let pit_universe = PointInTimeInstrumentUniverse::new(
        universe.snapshot_id.clone(),
        universe.universe.as_of_ms,
        members.clone(),
        market_context,
        evidence_state,
    )
    .map_err(|error| error.to_string())?;
    let mut bars_by_instrument: BTreeMap<String, BTreeMap<i64, FeatureMarketBar>> = BTreeMap::new();
    for component in &universe.components {
        let (_, bars) = inner
            .source
            .snapshot_for_user(&request.user_id, &component.snapshot_id)?;
        let instrument_id = format!(
            "{}:{}",
            component.dataset.instrument.venue.id, component.dataset.instrument.code
        );
        let by_time = bars_by_instrument.entry(instrument_id).or_default();
        for bar in bars {
            let close = adaq_data_core::next_bar_open_time_ms(bar.open_time_ms, universe.interval)
                .map_err(string)?;
            by_time.insert(close, FeatureMarketBar::from_ohlcv(bar));
        }
    }
    let mut observation_times = bars_by_instrument
        .values()
        .flat_map(|by_time| by_time.keys().copied())
        .collect::<Vec<_>>();
    observation_times.sort_unstable();
    observation_times.dedup();
    observation_times.retain(|time| {
        request.start_time_ms.is_none_or(|start| *time >= start)
            && request.end_time_ms.is_none_or(|end| *time < end)
    });
    let max_batches = request
        .max_events
        .unwrap_or(MAX_PREVIEW_CROSS_SECTIONAL_BATCHES)
        .min(MAX_PREVIEW_CROSS_SECTIONAL_BATCHES);
    let truncated = observation_times.len() > max_batches;
    observation_times.truncate(max_batches);
    let event_count = observation_times.len();
    let mut observations = Vec::new();
    for time in observation_times {
        let inputs = members
            .iter()
            .map(|member| {
                match bars_by_instrument
                    .get(member)
                    .and_then(|by_time| by_time.get(&time))
                {
                    Some(bar) => FeatureEvaluationInput::new(member, time, time, bar.clone()),
                    None => FeatureEvaluationInput::missing(member, time, time),
                }
            })
            .collect::<Vec<_>>();
        let event = FeatureInputEvent::cross_sectional_batch(time, pit_universe.clone(), inputs);
        observations.extend(
            evaluator
                .observe(event)
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(FeaturePreviewView {
        event_count,
        observations,
        truncated,
    })
}

fn evaluate_all(
    evaluator: &mut adaq_feature_engine::FeatureEvaluator,
    events: &[FeatureInputEvent],
) -> Result<Vec<FeatureObservation>, String> {
    let mut observations = Vec::new();
    for event in events {
        observations.extend(
            evaluator
                .observe(event.clone())
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(observations)
}
