use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Barrier},
    thread,
};

use adaq_feature_engine::{
    DefinitionDraft, FeatureDatasetCell, FeatureDatasetFilter, FeatureDatasetRowState,
    FeatureDefinition, FeatureEngineIdentity, FeatureEvaluationInput, FeatureInput,
    FeatureInputEvent, FeatureMarketBar, FeatureMaterializationRequest,
    FeatureMaterializationStore, FeatureNode, FeatureObservation, FeatureOperator, FeatureOutput,
    FeaturePlan, FeaturePlanDraft, FeatureReference, FeatureScope, FeatureSlot, FeatureSource,
    FeatureUnavailabilityReason, FittedArtifactBinding, MaterializationAttemptStatus,
    MaterializationStoreError, ObservationRange,
};
use rusqlite::Connection;
use tempfile::tempdir;
use uuid::Uuid;

fn plan() -> FeaturePlan {
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: Uuid::from_u128(100),
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
    .unwrap();
    let definition_hash = definition.definition_hash().to_owned();
    FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        slots: vec![FeatureSlot {
            name: "other".into(),
            source: FeatureSource::Market {
                field: "close".into(),
            },
            warmup_bars: 0,
        }],
        artifacts: vec![FittedArtifactBinding {
            artifact_id: "artifact-1".into(),
            eligible_at_ms: 0,
            fitted_output: FeatureReference {
                definition_hash,
                node_id: "return".into(),
                output_name: "return".into(),
            },
        }],
        engine_identity: FeatureEngineIdentity::for_tests(),
        ..FeaturePlanDraft::default()
    })
    .unwrap()
}

fn request_for(plan: &FeaturePlan, user_id: &str) -> FeatureMaterializationRequest {
    FeatureMaterializationRequest::new(
        user_id,
        plan.plan_hash(),
        "snapshot-1",
        "universe-1",
        ObservationRange {
            start_time_ms: 0,
            end_time_ms: 100,
        },
        BTreeMap::new(),
        7,
    )
    .unwrap()
}

fn observations() -> Vec<FeatureObservation> {
    vec![
        FeatureObservation::available("return", "BTC-USD", 10, 0.5, 10).unwrap(),
        FeatureObservation::available("other", "BTC-USD", 10, 1.5, 10).unwrap(),
        FeatureObservation::unavailable(
            "return",
            "BTC-USD",
            20,
            FeatureUnavailabilityReason::Warmup,
        )
        .unwrap(),
        FeatureObservation::unavailable(
            "other",
            "BTC-USD",
            20,
            FeatureUnavailabilityReason::MissingMarketInput,
        )
        .unwrap(),
    ]
}

#[test]
fn stage_events_uses_the_same_evaluator_as_stateful_observation() {
    let root = tempdir().unwrap();
    let store = FeatureMaterializationStore::open(root.path().join("research.sqlite"), root.path())
        .unwrap();
    let plan = plan();
    let pending = store
        .start_for_plan(request_for(&plan, "alice"), &plan)
        .unwrap();
    store.begin("alice", &pending.attempt_id).unwrap();
    let events = vec![
        FeatureInputEvent::observation(FeatureEvaluationInput::new(
            "BTC-USD",
            10,
            10,
            FeatureMarketBar::complete(10, "10", "10", "10", "10", "1", "10").unwrap(),
        )),
        FeatureInputEvent::observation(FeatureEvaluationInput::new(
            "BTC-USD",
            20,
            20,
            FeatureMarketBar::complete(20, "12", "12", "12", "12", "1", "12").unwrap(),
        )),
    ];
    store
        .stage_events("alice", &pending.attempt_id, &events, &[])
        .unwrap();
    let completed = store.publish("alice", &pending.attempt_id).unwrap();
    let dataset = store
        .dataset("alice", completed.dataset_id.as_deref().unwrap())
        .unwrap();
    let page = store
        .page(
            "alice",
            &dataset.dataset_id,
            FeatureDatasetFilter::default(),
            0,
        )
        .unwrap();
    assert_eq!(page.rows.len(), 2);
    assert!(matches!(
        page.rows[1].values["return"],
        FeatureDatasetCell::Available { value, .. } if (value - 0.2).abs() < 1e-12
    ));
}

fn publish_alice(
    store: &FeatureMaterializationStore,
    plan: &FeaturePlan,
) -> (MaterializationAttemptStatus, String, std::path::PathBuf) {
    let request = request_for(plan, "alice");
    let pending = store.start_for_plan(request, plan).unwrap();
    store.begin("alice", &pending.attempt_id).unwrap();
    store
        .stage(
            "alice",
            &pending.attempt_id,
            &["return", "other"],
            &observations(),
        )
        .unwrap();
    let completed = store.publish("alice", &pending.attempt_id).unwrap();
    let dataset_id = completed.dataset_id.clone().unwrap();
    let path = store.dataset("alice", &dataset_id).unwrap().parquet_path;
    (completed.status, dataset_id, path)
}

#[test]
fn materialization_publishes_immutable_wide_parquet_and_completed_metadata() {
    let root = tempdir().unwrap();
    let store = FeatureMaterializationStore::open(root.path().join("research.sqlite"), root.path())
        .unwrap();
    let plan = plan();
    let request = request_for(&plan, "alice");
    let pending = store.start_for_plan(request.clone(), &plan).unwrap();
    let coalesced = store.start_for_plan(request.clone(), &plan).unwrap();
    assert_eq!(coalesced.attempt_id, pending.attempt_id);
    let running = store.begin("alice", &pending.attempt_id).unwrap();
    assert_eq!(running.status, MaterializationAttemptStatus::Running);

    store
        .stage(
            "alice",
            &pending.attempt_id,
            &["return", "other"],
            &observations(),
        )
        .unwrap();

    let completed = store.publish("alice", &pending.attempt_id).unwrap();
    assert_eq!(completed.status, MaterializationAttemptStatus::Completed);
    let dataset = store
        .dataset("alice", completed.dataset_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(dataset.manifest.row_count, 2);
    assert_eq!(dataset.manifest.outputs[0].output_name, "return");
    assert_eq!(dataset.manifest.artifact_ids, vec!["artifact-1"]);
    assert_eq!(dataset.manifest.engine_identity, plan.engine_identity());
    assert!(dataset.parquet_path.is_file());
    assert_eq!(
        fs::read(&dataset.parquet_path).unwrap().len(),
        dataset.content_byte_size as usize
    );
    let moved_path = root.path().join("moved.parquet");
    fs::rename(&dataset.parquet_path, &moved_path).unwrap();
    fs::rename(&moved_path, &dataset.parquet_path).unwrap();

    let same = store.start_for_plan(request, &plan).unwrap();
    assert_eq!(same.attempt_id, pending.attempt_id);
    let summary = store.summary("alice", &dataset.dataset_id).unwrap();
    assert_eq!(summary[0].available_count, 1);
    assert_eq!(summary[0].unavailable_counts["warmup"], 1);
    assert_eq!(summary[0].mean, Some(0.5));
    let page = store
        .page(
            "alice",
            &dataset.dataset_id,
            FeatureDatasetFilter {
                output_name: Some("return".into()),
                state: Some(FeatureDatasetRowState::Unavailable),
                ..FeatureDatasetFilter::default()
            },
            0,
        )
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].observation_time_ms, 20);
}

#[test]
fn duplicate_and_incomplete_observations_are_rejected_before_publication() {
    let root = tempdir().unwrap();
    let store = FeatureMaterializationStore::with_connection(
        Connection::open_in_memory().unwrap(),
        root.path(),
    )
    .unwrap();
    let plan = plan();
    let pending = store
        .start_for_plan(request_for(&plan, "alice"), &plan)
        .unwrap();
    store.begin("alice", &pending.attempt_id).unwrap();
    assert!(matches!(
        store.stage("alice", &pending.attempt_id, &["return"], &observations()),
        Err(MaterializationStoreError::InvalidOutputSchema)
    ));
    let mut invalid_instrument = observations();
    invalid_instrument[0].instrument_id.clear();
    assert!(matches!(
        store.stage(
            "alice",
            &pending.attempt_id,
            &["return", "other"],
            &invalid_instrument
        ),
        Err(MaterializationStoreError::InvalidObservation(message))
            if message == "empty-instrument-id"
    ));
    let duplicate = vec![
        FeatureObservation::available("return", "BTC-USD", 10, 1.0, 10).unwrap(),
        FeatureObservation::available("return", "BTC-USD", 10, 2.0, 10).unwrap(),
    ];
    assert!(matches!(
        store.stage(
            "alice",
            &pending.attempt_id,
            &["return", "other"],
            &duplicate
        ),
        Err(MaterializationStoreError::DuplicateObservation)
    ));
    assert!(matches!(
        store.stage(
            "alice",
            &pending.attempt_id,
            &["return", "other"],
            &[FeatureObservation::available("return", "BTC-USD", 10, 1.0, 10).unwrap()]
        ),
        Err(MaterializationStoreError::IncompleteRows)
    ));
    let cancelled = store.cancel("alice", &pending.attempt_id).unwrap();
    assert_eq!(cancelled.status, MaterializationAttemptStatus::Cancelled);
    assert_eq!(cancelled.failure_code.as_deref(), Some("cancelled"));
    assert!(cancelled.diagnostic.is_some());
    assert!(cancelled.dataset_id.is_none());
}

#[test]
fn pending_attempts_use_enqueue_order() {
    let root = tempdir().unwrap();
    let store = FeatureMaterializationStore::with_connection(
        Connection::open_in_memory().unwrap(),
        root.path(),
    )
    .unwrap();
    let plan = plan();
    let first = store
        .start_for_plan(request_for(&plan, "alice"), &plan)
        .unwrap();
    let second = store
        .start_for_plan(request_for(&plan, "bob"), &plan)
        .unwrap();
    assert_eq!(
        store.next_pending().unwrap().unwrap().attempt_id,
        first.attempt_id
    );
    store
        .fail("alice", &first.attempt_id, "worker-error")
        .unwrap();
    assert_eq!(
        store.next_pending().unwrap().unwrap().attempt_id,
        second.attempt_id
    );
}

#[test]
fn staging_claim_allows_only_one_concurrent_writer() {
    let root = tempdir().unwrap();
    let database_path = root.path().join("research.sqlite");
    let plan = plan();
    let store = FeatureMaterializationStore::open(&database_path, root.path()).unwrap();
    let first_store = FeatureMaterializationStore::open(&database_path, root.path()).unwrap();
    let second_store = FeatureMaterializationStore::open(&database_path, root.path()).unwrap();
    let pending = store
        .start_for_plan(request_for(&plan, "alice"), &plan)
        .unwrap();
    store.begin("alice", &pending.attempt_id).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let attempt_id = pending.attempt_id.clone();
    let (first, second) = thread::scope(|scope| {
        let first_barrier = barrier.clone();
        let first_attempt_id = attempt_id.clone();
        let first = scope.spawn(move || {
            first_barrier.wait();
            first_store.stage(
                "alice",
                &first_attempt_id,
                &["return", "other"],
                &observations(),
            )
        });
        let second_barrier = barrier.clone();
        let second = scope.spawn(move || {
            second_barrier.wait();
            second_store.stage("alice", &attempt_id, &["return", "other"], &observations())
        });
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_eq!(
        usize::from(first.is_ok() as u8) + usize::from(second.is_ok() as u8),
        1,
        "first={first:?} second={second:?}"
    );
    assert!(matches!(
        (first, second),
        (Err(MaterializationStoreError::InvalidTransition), _)
            | (_, Err(MaterializationStoreError::InvalidTransition))
    ));
    store.cancel("alice", &pending.attempt_id).unwrap();
}

#[test]
fn failed_attempts_retry_with_retained_source_and_pending_survives_restart() {
    let root = tempdir().unwrap();
    let database_path = root.path().join("research.sqlite");
    let plan = plan();
    let pending_id = {
        let store = FeatureMaterializationStore::open(&database_path, root.path()).unwrap();
        let pending = store
            .start_for_plan(request_for(&plan, "alice"), &plan)
            .unwrap();
        let running = store.begin("alice", &pending.attempt_id).unwrap();
        store
            .stage(
                "alice",
                &running.attempt_id,
                &["return", "other"],
                &observations(),
            )
            .unwrap();
        let failed = store
            .fail("alice", &running.attempt_id, "worker-error")
            .unwrap();
        let retry = store.retry("alice", &failed.attempt_id).unwrap();
        let coalesced_retry = store.retry("alice", &failed.attempt_id).unwrap();
        assert_eq!(coalesced_retry.attempt_id, retry.attempt_id);
        assert_eq!(
            retry.source_attempt_id.as_deref(),
            Some(failed.attempt_id.as_str())
        );
        retry.attempt_id
    };
    let store = FeatureMaterializationStore::open(&database_path, root.path()).unwrap();
    let attempts = store.attempts("alice").unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].status, MaterializationAttemptStatus::Failed);
    assert_eq!(attempts[1].attempt_id, pending_id);
    assert_eq!(attempts[1].status, MaterializationAttemptStatus::Pending);
}

#[test]
fn startup_marks_running_interrupted_and_removes_only_its_staging_file() {
    let root = tempdir().unwrap();
    let database_path = root.path().join("research.sqlite");
    let plan = plan();
    let (attempt_id, staging_path) = {
        let store = FeatureMaterializationStore::open(&database_path, root.path()).unwrap();
        let pending = store
            .start_for_plan(request_for(&plan, "alice"), &plan)
            .unwrap();
        store.begin("alice", &pending.attempt_id).unwrap();
        store
            .stage(
                "alice",
                &pending.attempt_id,
                &["return", "other"],
                &observations(),
            )
            .unwrap();
        (
            pending.attempt_id.clone(),
            root.path()
                .join("staging")
                .join(format!("{}.parquet", pending.attempt_id)),
        )
    };
    assert!(staging_path.is_file());
    let store = FeatureMaterializationStore::open(&database_path, root.path()).unwrap();
    let interrupted = store.attempt("alice", &attempt_id).unwrap();
    assert_eq!(interrupted.status, MaterializationAttemptStatus::Failed);
    assert_eq!(interrupted.failure_code.as_deref(), Some("interrupted"));
    assert!(!staging_path.exists());
}

#[test]
fn content_hash_corruption_is_not_consumable_and_dataset_references_lock_deletion() {
    let root = tempdir().unwrap();
    let store = FeatureMaterializationStore::with_connection(
        Connection::open_in_memory().unwrap(),
        root.path(),
    )
    .unwrap();
    let plan = plan();
    let (_, dataset_id, parquet_path) = publish_alice(&store, &plan);
    let mut bytes = fs::read(&parquet_path).unwrap();
    bytes[0] ^= 1;
    fs::write(&parquet_path, bytes).unwrap();
    assert!(matches!(
        store.summary("alice", &dataset_id),
        Err(MaterializationStoreError::DatasetContentCollision)
    ));

    let root = tempdir().unwrap();
    let store = FeatureMaterializationStore::with_connection(
        Connection::open_in_memory().unwrap(),
        root.path(),
    )
    .unwrap();
    let (_, dataset_id, parquet_path) = publish_alice(&store, &plan);
    assert!(store.dataset("bob", &dataset_id).is_err());
    store
        .reference_dataset("alice", &dataset_id, "bob", "run-1")
        .unwrap();
    assert!(store.dataset("bob", &dataset_id).is_ok());
    assert!(matches!(
        store.delete_dataset("alice", &dataset_id),
        Err(MaterializationStoreError::DatasetReferenced)
    ));
    store
        .unreference_dataset("bob", &dataset_id, "run-1")
        .unwrap();
    store.delete_dataset("alice", &dataset_id).unwrap();
    assert!(store.dataset("bob", &dataset_id).is_ok());
    store.delete_dataset("bob", &dataset_id).unwrap();
    assert!(!parquet_path.exists());
}

#[test]
fn incompatible_feature_schema_requires_explicit_reset_without_deletion() {
    let root = tempdir().unwrap();
    let database_path = root.path().join("research.sqlite");
    let database = Connection::open(&database_path).unwrap();
    database
        .execute(
            "CREATE TABLE feature_materialization_attempts (attempt_id TEXT PRIMARY KEY)",
            [],
        )
        .unwrap();
    drop(database);
    let error = match FeatureMaterializationStore::open(&database_path, root.path()) {
        Ok(_) => panic!("incompatible schema was accepted"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        MaterializationStoreError::ResetRequired { .. }
    ));
    let database = Connection::open(&database_path).unwrap();
    let table_count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'feature_materialization_attempts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_count, 1);
}
