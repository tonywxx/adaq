//! Bounded transient Draft Preview over immutable Snapshot evidence.
//!
//! Preview uses the production Feature Engine, never fits, creates no
//! evidence identity, and persists nothing. Cross-Sectional work always
//! observes complete Point-in-Time Universe batches; Pointwise and
//! Time-Series previews may be bounded by Observation Time selection.

use adaq_feature_engine::{
    FeatureDefinition, FeatureEngine, FeatureInputEvent, FeatureObservation, FeaturePlan,
    FeaturePlanDraft, FeatureScope, FittedArtifactBinding, FittedTransformationArtifact,
    ObservationRange,
};

use super::{
    FeaturePreviewRequest, FeaturePreviewView, FeaturesInner, MAX_PREVIEW_CROSS_SECTIONAL_BATCHES,
    MAX_PREVIEW_OBSERVATIONS, runner, store,
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
    let snapshot_id = request
        .snapshot_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or("Cross-Sectional Feature Preview requires a Snapshot")?;
    let range = ObservationRange {
        start_time_ms: request.start_time_ms.unwrap_or(i64::MIN),
        end_time_ms: request.end_time_ms.unwrap_or(i64::MAX),
    };
    let mut events = runner::cross_sectional_events(
        inner,
        &request.user_id,
        Some(snapshot_id),
        universe_id,
        &range,
        valuation_currency,
    )?;
    let max_batches = request
        .max_events
        .unwrap_or(MAX_PREVIEW_CROSS_SECTIONAL_BATCHES)
        .min(MAX_PREVIEW_CROSS_SECTIONAL_BATCHES);
    let truncated = events.len() > max_batches;
    events.truncate(max_batches);
    let observations = evaluate_all(evaluator, &events)?;
    Ok(FeaturePreviewView {
        event_count: events.len(),
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
