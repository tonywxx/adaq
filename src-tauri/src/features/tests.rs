use super::*;
use crate::{local_research::LocalResearchState, watchlist::WatchlistDb};
use adaq_backtest_core::{
    MarketDataUniverseSnapshot, SnapshotDatasetBinding, SnapshotProvenance,
    SnapshotUniverseBinding, UniverseSnapshotComponent,
};
use adaq_data_core::market::{InstrumentId, Venue};
use adaq_data_core::{BarGap, BarInterval, BarSeries, OhlcvBar};
use adaq_feature_engine::{
    DefinitionDraft, FeatureDefinition, FeatureEngineIdentity, FeatureInput, FeatureNode,
    FeatureOperator, FeatureOutput, FeaturePlan, FeaturePlanDraft, FeatureReference, FeatureScope,
    FittingAlgorithm, FittingScope, MarketField, MaterializationAttemptStatus, ObservationRange,
    TransformationFittingProtocol, TransformationFittingProtocolDraft,
};
use rust_decimal::Decimal;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

const HOUR: i64 = 3_600_000;

fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "adaq-features-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ))
}

/// Alice owns one OKX Spot Snapshot with six Closed Bars and one Bar Gap.
fn setup(
    name: &str,
) -> (
    PathBuf,
    Arc<LocalResearchState>,
    adaq_backtest_core::MarketDataSnapshot,
) {
    let root = root(name);
    let state = LocalResearchState::open(&root).unwrap();
    let bars = [0, 1, 2, 6, 7, 8]
        .into_iter()
        .enumerate()
        .map(|(index, hour)| {
            let value = Decimal::from(i64::try_from(index + 1).unwrap());
            OhlcvBar {
                open_time_ms: hour * HOUR,
                open: value,
                high: value,
                low: value,
                close: value,
                base_volume: Decimal::ONE,
                quote_volume: value,
            }
        })
        .collect();
    let snapshot = state
        .persist_snapshot_for_user(
            "alice",
            &BarSeries {
                src: "okx".into(),
                code: "BTC-USDT".into(),
                interval: BarInterval::OneHour,
                bars,
                gaps: vec![BarGap {
                    start_time_ms: 3 * HOUR,
                    end_time_ms: 6 * HOUR,
                }],
            },
        )
        .unwrap();
    (root, state, snapshot)
}

fn cross_sectional_setup(name: &str) -> (PathBuf, Arc<LocalResearchState>, String, String, String) {
    let root = root(name);
    let state = LocalResearchState::open(&root).unwrap();
    let venue = Venue::crypto_spot("okx").unwrap();
    let make_component = |code: &str, values: [i64; 3]| {
        let instrument = InstrumentId::new(venue.clone(), code).unwrap();
        let dataset = SnapshotDatasetBinding {
            instrument: instrument.clone(),
            source_id: format!("test-source-{code}"),
            source_revision: 1,
            canonical_id: Some(format!("test-canonical-{code}")),
            derived_id: None,
            quality_report_id: format!("test-quality-{code}"),
            content_sha256: format!("test-content-{code}"),
        };
        let snapshot = state
            .snapshots
            .persist_for_user_with_provenance(
                "alice",
                &BarSeries {
                    src: venue.id.clone(),
                    code: code.into(),
                    interval: BarInterval::OneHour,
                    bars: values
                        .into_iter()
                        .map(|value| OhlcvBar {
                            open_time_ms: value,
                            open: Decimal::from(value + 10),
                            high: Decimal::from(value + 10),
                            low: Decimal::from(value + 10),
                            close: Decimal::from(value + 10),
                            base_volume: Decimal::ONE,
                            quote_volume: Decimal::from(value + 10),
                        })
                        .collect(),
                    gaps: vec![],
                },
                Some(SnapshotProvenance {
                    venue: venue.clone(),
                    datasets: vec![dataset.clone()],
                    quality_report_ids: vec![dataset.quality_report_id.clone()],
                    calendar_snapshot_ids: vec!["test-calendar".into()],
                    provider_capability_snapshots: vec![],
                    universe: None,
                    derivation_algorithm_version: None,
                }),
            )
            .unwrap();
        (snapshot, dataset, instrument)
    };
    let (btc, btc_dataset, btc_instrument) = make_component("BTC-USDT", [0, HOUR, 2 * HOUR]);
    let (eth, eth_dataset, eth_instrument) = make_component("ETH-USDT", [0, HOUR, 2 * HOUR]);
    let universe = state
        .snapshots
        .persist_universe_for_user(
            "alice",
            MarketDataUniverseSnapshot {
                snapshot_id: String::new(),
                venue,
                interval: BarInterval::OneHour,
                start_time_ms: btc.start_time_ms,
                end_time_ms: btc.end_time_ms,
                universe: SnapshotUniverseBinding {
                    universe_id: "test-pit-universe".into(),
                    as_of_ms: 0,
                    evidence_state: "observed".into(),
                    evidence_reasons: vec!["test-observed-membership".into()],
                    coverage_start_ms: Some(btc.start_time_ms),
                    coverage_end_ms: Some(btc.end_time_ms),
                    instruments: vec![btc_instrument, eth_instrument],
                },
                components: vec![
                    UniverseSnapshotComponent {
                        snapshot_id: btc.snapshot_id.clone(),
                        dataset: btc_dataset,
                    },
                    UniverseSnapshotComponent {
                        snapshot_id: eth.snapshot_id.clone(),
                        dataset: eth_dataset,
                    },
                ],
                quality_report_ids: vec![
                    "test-quality-BTC-USDT".into(),
                    "test-quality-ETH-USDT".into(),
                ],
                calendar_snapshot_ids: vec!["test-calendar".into()],
                provider_capability_snapshots: vec![],
                content_sha256: String::new(),
            },
        )
        .unwrap();
    (
        root,
        state,
        btc.snapshot_id,
        eth.snapshot_id,
        universe.snapshot_id,
    )
}

fn cross_sectional_rank_draft() -> DefinitionDraft {
    DefinitionDraft {
        definition_id: Uuid::from_u128(8),
        revision: 1,
        scope: FeatureScope::CrossSectional,
        nodes: vec![FeatureNode {
            id: "rank".into(),
            operator: FeatureOperator::CrossSectionalRank,
            scope: FeatureScope::CrossSectional,
            inputs: vec![FeatureInput::Market {
                field: MarketField::Close,
            }],
            parameters: BTreeMap::new(),
            warmup_bars: 0,
        }],
        outputs: vec![FeatureOutput {
            name: "rank".into(),
            node_id: "rank".into(),
        }],
    }
}

fn return_draft() -> DefinitionDraft {
    DefinitionDraft {
        definition_id: Uuid::from_u128(7),
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
    }
}

fn native_plan(definitions: Vec<FeatureDefinition>) -> FeaturePlan {
    FeaturePlan::freeze(FeaturePlanDraft {
        definitions,
        engine_identity: FeatureEngineIdentity::native().unwrap(),
        ..FeaturePlanDraft::default()
    })
    .unwrap()
}

fn plan_draft(definitions: Vec<FeatureDefinition>) -> FeaturePlanDraft {
    FeaturePlanDraft {
        definitions,
        engine_identity: FeatureEngineIdentity::native().unwrap(),
        ..FeaturePlanDraft::default()
    }
}

fn materialization_request(
    user_id: &str,
    plan: &FeaturePlan,
    snapshot_id: &str,
) -> FeatureMaterializationRequest {
    FeatureMaterializationRequest::new(
        user_id,
        plan.plan_hash(),
        snapshot_id,
        "universe-1",
        ObservationRange {
            start_time_ms: 0,
            end_time_ms: 10 * HOUR,
        },
        BTreeMap::new(),
        7,
    )
    .unwrap()
}

fn fitting_protocol_draft(
    definition: &FeatureDefinition,
    snapshot_id: &str,
    minimum_samples: u64,
) -> TransformationFittingProtocolDraft {
    TransformationFittingProtocolDraft {
        input_feature: FeatureReference {
            definition_hash: definition.definition_hash().into(),
            node_id: "return".into(),
            output_name: "return".into(),
        },
        fitted_node_id: "return".into(),
        fitted_output: FeatureReference {
            definition_hash: definition.definition_hash().into(),
            node_id: "return".into(),
            output_name: "return-standardized".into(),
        },
        snapshot_id: snapshot_id.into(),
        point_in_time_universe_id: "universe-1".into(),
        valuation_currency: String::new(),
        fitting_scope: FittingScope::PooledUniverse,
        fitting_window: ObservationRange {
            start_time_ms: 0,
            end_time_ms: 10 * HOUR,
        },
        algorithm: FittingAlgorithm::Standardization,
        minimum_samples,
        engine_identity: FeatureEngineIdentity::for_tests(),
    }
}

fn wait_for_materialization(
    state: &LocalResearchState,
    user_id: &str,
    attempt_id: &str,
    expected: MaterializationAttemptStatus,
) -> MaterializationAttempt {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let attempt = state
            .features
            .list_materialization_attempts(FeatureUserRequest {
                user_id: user_id.into(),
            })
            .unwrap()
            .into_iter()
            .find(|attempt| attempt.attempt_id == attempt_id)
            .unwrap();
        if attempt.status == expected {
            return attempt;
        }
        assert!(
            !matches!(
                attempt.status,
                MaterializationAttemptStatus::Completed
                    | MaterializationAttemptStatus::Failed
                    | MaterializationAttemptStatus::Cancelled
            ),
            "Attempt {attempt_id} reached {:?} before {expected:?} ({:?}: {:?})",
            attempt.status,
            attempt.failure_code,
            attempt.diagnostic
        );
        assert!(
            Instant::now() < deadline,
            "Attempt {attempt_id} did not reach {expected:?}: {:?}",
            attempt.status
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_fitting(
    state: &LocalResearchState,
    user_id: &str,
    attempt_id: &str,
    expected: FeatureAttemptStatus,
) -> FittingAttemptView {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let attempt = state
            .features
            .list_fitting_attempts(FeatureUserRequest {
                user_id: user_id.into(),
            })
            .unwrap()
            .into_iter()
            .find(|attempt| attempt.attempt_id == attempt_id)
            .unwrap();
        if attempt.status == expected {
            return attempt;
        }
        assert!(
            !matches!(
                attempt.status,
                FeatureAttemptStatus::Completed
                    | FeatureAttemptStatus::Failed
                    | FeatureAttemptStatus::Cancelled
            ),
            "Attempt {attempt_id} reached {:?} before {expected:?} ({:?}: {:?})",
            attempt.status,
            attempt.failure_code,
            attempt.diagnostic
        );
        assert!(
            Instant::now() < deadline,
            "Attempt {attempt_id} did not reach {expected:?}: {:?} ({:?}: {:?})",
            attempt.status,
            attempt.failure_code,
            attempt.diagnostic
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Holds the runner at each Attempt start until the test releases it, and
/// records the observed execution order.
struct RunnerHold {
    started: Arc<(Mutex<Vec<(String, String)>>, Condvar)>,
    release: Arc<AtomicBool>,
}

impl RunnerHold {
    fn install(state: &LocalResearchState) -> Self {
        let started = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let release = Arc::new(AtomicBool::new(false));
        let hook_started = started.clone();
        let hook_release = release.clone();
        *state.features.inner.attempt_started_hook.lock().unwrap() =
            Some(Arc::new(move |kind: &str, attempt_id: &str| {
                {
                    let mut attempts = hook_started.0.lock().unwrap();
                    attempts.push((kind.to_owned(), attempt_id.to_owned()));
                    hook_started.1.notify_all();
                }
                while !hook_release.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }));
        Self { started, release }
    }

    fn wait_for(&self, kind: &str, attempt_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut attempts = self.started.0.lock().unwrap();
        loop {
            if attempts.iter().any(|(observed_kind, observed_id)| {
                observed_kind == kind && observed_id == attempt_id
            }) {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                remaining > Duration::ZERO,
                "runner never started {kind} {attempt_id}"
            );
            let (next, _) = self
                .started
                .1
                .wait_timeout(attempts, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            attempts = next;
        }
    }

    fn release(&self) {
        self.release.store(true, Ordering::Relaxed);
    }
}

impl Drop for RunnerHold {
    fn drop(&mut self) {
        self.release();
    }
}

#[test]
fn definitions_are_user_scoped_and_presentation_never_changes_the_hash() {
    let (root, state, _snapshot) = setup("definitions");
    let alice = state
        .features
        .publish_definition(DefinitionPublishRequest {
            user_id: "alice".into(),
            draft: return_draft(),
            name: "Alice Return".into(),
            description: "alice copy".into(),
            tags: vec!["momentum".into()],
        })
        .unwrap();
    assert_eq!(alice.name, "Alice Return");
    assert_eq!(alice.tags, vec!["momentum".to_string()]);
    let bob = state
        .features
        .publish_definition(DefinitionPublishRequest {
            user_id: "bob".into(),
            draft: return_draft(),
            name: "Bob Return".into(),
            description: String::new(),
            tags: vec![],
        })
        .unwrap();
    assert_eq!(
        alice.definition_hash, bob.definition_hash,
        "display metadata must never change the semantic hash"
    );
    assert_eq!(bob.name, "Bob Return");
    let alice_list = state
        .features
        .list_definitions(FeatureUserRequest {
            user_id: "alice".into(),
        })
        .unwrap();
    let bob_list = state
        .features
        .list_definitions(FeatureUserRequest {
            user_id: "bob".into(),
        })
        .unwrap();
    assert_eq!(alice_list.len(), 1);
    assert_eq!(bob_list.len(), 1);
    assert_eq!(alice_list[0].name, "Alice Return");
    assert_eq!(bob_list[0].name, "Bob Return");
    assert_eq!(alice_list[0].definition_json, bob_list[0].definition_json);
    assert!(
        state
            .features
            .get_definition(DefinitionIdRequest {
                user_id: "bob".into(),
                definition_hash: alice.definition_hash.clone(),
            })
            .is_ok()
    );
    // A higher revision published by one User must not hide the revision
    // another User published for the same Definition family.
    let mut bob_revision = return_draft();
    bob_revision.revision = 2;
    bob_revision.nodes[0].warmup_bars = 2;
    state
        .features
        .publish_definition(DefinitionPublishRequest {
            user_id: "bob".into(),
            draft: bob_revision,
            name: "Bob Return v2".into(),
            description: String::new(),
            tags: vec![],
        })
        .unwrap();
    let alice_list = state
        .features
        .list_definitions(FeatureUserRequest {
            user_id: "alice".into(),
        })
        .unwrap();
    assert_eq!(alice_list.len(), 1);
    assert_eq!(alice_list[0].revision, 1, "alice keeps her own revision");
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn definition_revision_must_increase_and_validation_is_typed() {
    let (root, state, _snapshot) = setup("definition-revision");
    state
        .features
        .publish_definition(DefinitionPublishRequest {
            user_id: "alice".into(),
            draft: return_draft(),
            name: String::new(),
            description: String::new(),
            tags: vec![],
        })
        .unwrap();
    let mut same_revision = return_draft();
    same_revision.nodes[0].warmup_bars = 2;
    let error = state
        .features
        .publish_definition(DefinitionPublishRequest {
            user_id: "alice".into(),
            draft: same_revision.clone(),
            name: String::new(),
            description: String::new(),
            tags: vec![],
        })
        .unwrap_err();
    assert!(error.contains("revision must increase"), "{error}");
    same_revision.revision = 2;
    let revised = state
        .features
        .publish_definition(DefinitionPublishRequest {
            user_id: "alice".into(),
            draft: same_revision,
            name: String::new(),
            description: String::new(),
            tags: vec![],
        })
        .unwrap();
    assert_eq!(revised.revision, 2);
    let listed = state
        .features
        .list_definitions(FeatureUserRequest {
            user_id: "alice".into(),
        })
        .unwrap();
    assert_eq!(listed.len(), 1, "listing shows one family row");
    assert_eq!(listed[0].revision, 2);

    let invalid = state
        .features
        .validate_definition_draft(DefinitionDraftRequest {
            user_id: "alice".into(),
            draft: DefinitionDraft {
                outputs: vec![],
                ..return_draft()
            },
        })
        .unwrap();
    assert!(!invalid.valid);
    assert!(!invalid.issues.is_empty());
    let valid = state
        .features
        .validate_definition_draft(DefinitionDraftRequest {
            user_id: "alice".into(),
            draft: return_draft(),
        })
        .unwrap();
    assert!(valid.valid && valid.issues.is_empty());
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_is_bounded_transient_and_fits_nothing() {
    let (root, state, snapshot) = setup("preview");
    let preview = state
        .features
        .preview_definition_draft(FeaturePreviewRequest {
            user_id: "alice".into(),
            draft: return_draft(),
            snapshot_id: Some(snapshot.snapshot_id.clone()),
            universe_id: None,
            valuation_currency: None,
            start_time_ms: None,
            end_time_ms: None,
            max_events: Some(2),
            artifact_ids: vec![],
        })
        .unwrap();
    assert_eq!(preview.event_count, 2);
    assert!(preview.truncated);
    assert!(!preview.observations.is_empty());
    let full = state
        .features
        .preview_definition_draft(FeaturePreviewRequest {
            user_id: "alice".into(),
            draft: return_draft(),
            snapshot_id: Some(snapshot.snapshot_id.clone()),
            universe_id: None,
            valuation_currency: None,
            start_time_ms: None,
            end_time_ms: None,
            max_events: None,
            artifact_ids: vec![],
        })
        .unwrap();
    assert!(!full.truncated);
    assert!(full.event_count > preview.event_count);
    // Preview is transient: it creates no Definitions, Attempts, Artifacts,
    // or Datasets.
    assert!(
        state
            .features
            .list_definitions(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .list_fitting_attempts(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .list_materialization_attempts(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .list_datasets(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        state
            .features
            .preview_definition_draft(FeaturePreviewRequest {
                user_id: "alice".into(),
                draft: return_draft(),
                snapshot_id: None,
                universe_id: None,
                valuation_currency: None,
                start_time_ms: None,
                end_time_ms: None,
                max_events: None,
                artifact_ids: vec![],
            })
            .unwrap_err(),
        "Feature Preview requires a Snapshot"
    );
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cross_sectional_preview_fitting_and_materialization_bind_complete_pit_batches() {
    let (root, state, snapshot_id, _second_snapshot_id, universe_id) =
        cross_sectional_setup("cross-sectional");
    let definition = FeatureDefinition::freeze(cross_sectional_rank_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    let preview = state
        .features
        .preview_definition_draft(FeaturePreviewRequest {
            user_id: "alice".into(),
            draft: cross_sectional_rank_draft(),
            snapshot_id: Some(snapshot_id.clone()),
            universe_id: Some(universe_id.clone()),
            valuation_currency: Some("USDT".into()),
            start_time_ms: Some(HOUR),
            end_time_ms: Some(4 * HOUR),
            max_events: None,
            artifact_ids: vec![],
        })
        .unwrap();
    assert_eq!(preview.event_count, 3);
    assert_eq!(preview.observations.len(), 6);
    assert!(
        preview
            .observations
            .iter()
            .all(|observation| observation.cross_sectional_coverage.is_some())
    );

    let request = FeatureMaterializationRequest::new(
        "alice",
        plan.plan_hash(),
        snapshot_id.clone(),
        universe_id.clone(),
        ObservationRange {
            start_time_ms: HOUR,
            end_time_ms: 4 * HOUR,
        },
        BTreeMap::new(),
        9,
    )
    .map(|mut request| {
        request.valuation_currency = "USDT".into();
        request
    })
    .unwrap();
    let started = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "alice".into(),
            request: request.clone(),
            plan: plan_draft(vec![definition.clone()]),
        })
        .unwrap();
    let completed = wait_for_materialization(
        &state,
        "alice",
        &started.attempt_id,
        MaterializationAttemptStatus::Completed,
    );
    let dataset = state
        .features
        .list_datasets(FeatureUserRequest {
            user_id: "alice".into(),
        })
        .unwrap()
        .into_iter()
        .find(|dataset| dataset.dataset_id == completed.dataset_id.clone().unwrap())
        .unwrap();
    assert_eq!(dataset.manifest.request.valuation_currency, "USDT");
    assert_eq!(dataset.manifest.row_count, 6);

    let missing_currency = FeatureMaterializationRequest {
        valuation_currency: String::new(),
        ..request
    };
    assert_eq!(
        state
            .features
            .start_materialization(FeatureMaterializationStartRequest {
                user_id: "alice".into(),
                request: missing_currency,
                plan: plan_draft(vec![definition.clone()]),
            })
            .unwrap_err(),
        "cross-sectional-feature-valuation-currency-required"
    );

    let protocol = TransformationFittingProtocolDraft {
        input_feature: FeatureReference {
            definition_hash: definition.definition_hash().into(),
            node_id: "rank".into(),
            output_name: "rank".into(),
        },
        fitted_node_id: "rank".into(),
        fitted_output: FeatureReference {
            definition_hash: definition.definition_hash().into(),
            node_id: "rank".into(),
            output_name: "rank-standardized".into(),
        },
        snapshot_id,
        point_in_time_universe_id: universe_id,
        valuation_currency: "USDT".into(),
        fitting_scope: FittingScope::PooledUniverse,
        fitting_window: ObservationRange {
            start_time_ms: HOUR,
            end_time_ms: 4 * HOUR,
        },
        algorithm: FittingAlgorithm::Standardization,
        minimum_samples: 2,
        engine_identity: FeatureEngineIdentity::for_tests(),
    };
    let fitting = state
        .features
        .start_fitting(FeatureFittingStartRequest {
            user_id: "alice".into(),
            protocol,
            plan: plan_draft(vec![definition]),
        })
        .unwrap();
    let fitting_completed = wait_for_fitting(
        &state,
        "alice",
        &fitting.attempt_id,
        FeatureAttemptStatus::Completed,
    );
    assert!(fitting_completed.artifact_id.is_some());
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fitting_publishes_an_artifact_and_coalesces_duplicates() {
    let (root, state, snapshot) = setup("fitting-complete");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let request = FeatureFittingStartRequest {
        user_id: "alice".into(),
        protocol: fitting_protocol_draft(&definition, &snapshot.snapshot_id, 2),
        plan: plan_draft(vec![definition]),
    };
    let started = state.features.start_fitting(request.clone()).unwrap();
    let completed = wait_for_fitting(
        &state,
        "alice",
        &started.attempt_id,
        FeatureAttemptStatus::Completed,
    );
    let artifact_id = completed.artifact_id.clone().unwrap();
    assert!(completed.progress_completed > 0);
    assert_eq!(completed.progress_completed, completed.progress_total);
    let artifact = state
        .features
        .get_artifact(FeatureArtifactRequest {
            user_id: "alice".into(),
            artifact_id: artifact_id.clone(),
        })
        .unwrap();
    assert_eq!(artifact.artifact_id, artifact_id);
    // An exact effective Protocol coalesces onto the Completed Attempt.
    let coalesced = state.features.start_fitting(request).unwrap();
    assert_eq!(coalesced.attempt_id, started.attempt_id);
    // Other Users cannot see the Attempt or the Artifact.
    assert!(
        state
            .features
            .list_fitting_attempts(FeatureUserRequest {
                user_id: "bob".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .get_artifact(FeatureArtifactRequest {
                user_id: "bob".into(),
                artifact_id,
            })
            .is_err()
    );
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fitting_insufficient_samples_fails_and_retry_keeps_source_evidence() {
    let (root, state, snapshot) = setup("fitting-retry");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let request = FeatureFittingStartRequest {
        user_id: "alice".into(),
        protocol: fitting_protocol_draft(&definition, &snapshot.snapshot_id, 100),
        plan: plan_draft(vec![definition]),
    };
    let started = state.features.start_fitting(request).unwrap();
    let failed = wait_for_fitting(
        &state,
        "alice",
        &started.attempt_id,
        FeatureAttemptStatus::Failed,
    );
    assert_eq!(failed.failure_code.as_deref(), Some("insufficient-samples"));
    assert!(
        failed
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("\"eligibleSamples\":4"),
        "{:?}",
        failed.diagnostic
    );
    assert_eq!(
        state
            .features
            .retry_fitting_attempt(FeatureAttemptRequest {
                user_id: "bob".into(),
                attempt_id: started.attempt_id.clone(),
            })
            .unwrap_err(),
        "Feature Fitting Attempt not found"
    );
    let retried = state
        .features
        .retry_fitting_attempt(FeatureAttemptRequest {
            user_id: "alice".into(),
            attempt_id: started.attempt_id.clone(),
        })
        .unwrap();
    assert_ne!(retried.attempt_id, started.attempt_id);
    assert_eq!(
        retried.source_attempt_id.as_deref(),
        Some(started.attempt_id.as_str())
    );
    let failed_again = wait_for_fitting(
        &state,
        "alice",
        &retried.attempt_id,
        FeatureAttemptStatus::Failed,
    );
    assert_eq!(
        failed_again.failure_code.as_deref(),
        Some("insufficient-samples")
    );
    // A Completed Attempt cannot be retried.
    let completed_request = FeatureFittingStartRequest {
        protocol: fitting_protocol_draft(
            &FeatureDefinition::freeze(return_draft()).unwrap(),
            &snapshot.snapshot_id,
            2,
        ),
        ..FeatureFittingStartRequest {
            user_id: "alice".into(),
            protocol: fitting_protocol_draft(
                &FeatureDefinition::freeze(return_draft()).unwrap(),
                &snapshot.snapshot_id,
                2,
            ),
            plan: plan_draft(vec![FeatureDefinition::freeze(return_draft()).unwrap()]),
        }
    };
    let completed = state.features.start_fitting(completed_request).unwrap();
    wait_for_fitting(
        &state,
        "alice",
        &completed.attempt_id,
        FeatureAttemptStatus::Completed,
    );
    assert_eq!(
        state
            .features
            .retry_fitting_attempt(FeatureAttemptRequest {
                user_id: "alice".into(),
                attempt_id: completed.attempt_id,
            })
            .unwrap_err(),
        "Feature Fitting Attempt cannot be retried"
    );
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn materialization_completes_a_dataset_and_reuses_completed_evidence() {
    let (root, state, snapshot) = setup("materialization-complete");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    let request = FeatureMaterializationStartRequest {
        user_id: "alice".into(),
        request: materialization_request("alice", &plan, &snapshot.snapshot_id),
        plan: plan_draft(vec![definition]),
    };
    let started = state
        .features
        .start_materialization(request.clone())
        .unwrap();
    assert_eq!(started.status, MaterializationAttemptStatus::Pending);
    let completed = wait_for_materialization(
        &state,
        "alice",
        &started.attempt_id,
        MaterializationAttemptStatus::Completed,
    );
    assert_eq!(completed.progress_completed, completed.progress_total);
    assert!(completed.progress_total > 0);
    let dataset_id = completed.dataset_id.clone().unwrap();
    let coalesced = state.features.start_materialization(request).unwrap();
    assert_eq!(coalesced.attempt_id, started.attempt_id);

    let dataset = state
        .features
        .get_dataset(FeatureDatasetRequest {
            user_id: "alice".into(),
            dataset_id: dataset_id.clone(),
        })
        .unwrap();
    assert_eq!(dataset.manifest.row_count, 7);
    let summary = state
        .features
        .dataset_summary(FeatureDatasetRequest {
            user_id: "alice".into(),
            dataset_id: dataset_id.clone(),
        })
        .unwrap();
    assert_eq!(summary[0].available_count, 4);
    assert_eq!(summary[0].unavailable_counts["warmup"], 2);
    assert_eq!(summary[0].unavailable_counts["bar-gap"], 1);
    let page = state
        .features
        .dataset_rows(FeatureDatasetRowsRequest {
            user_id: "alice".into(),
            dataset_id: dataset_id.clone(),
            filter: FeatureDatasetFilter::default(),
            offset: 0,
        })
        .unwrap();
    assert_eq!(page.rows.len(), 7);
    // User isolation on Dataset reads.
    assert!(
        state
            .features
            .list_datasets(FeatureUserRequest {
                user_id: "bob".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .get_dataset(FeatureDatasetRequest {
                user_id: "bob".into(),
                dataset_id,
            })
            .is_err()
    );
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_feature_dataset_establishes_a_user_scoped_factor_context() {
    let (root, state, snapshot_id, _other_snapshot_id, universe_id) =
        cross_sectional_setup("factor-context");
    let definition = FeatureDefinition::freeze(cross_sectional_rank_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    let mut feature_request = FeatureMaterializationRequest::new(
        "alice",
        plan.plan_hash(),
        &snapshot_id,
        &universe_id,
        ObservationRange {
            start_time_ms: 0,
            end_time_ms: 3 * HOUR,
        },
        BTreeMap::new(),
        7,
    )
    .unwrap();
    feature_request.valuation_currency = "USDT".into();
    let request = FeatureMaterializationStartRequest {
        user_id: "alice".into(),
        request: feature_request,
        plan: plan_draft(vec![definition]),
    };
    let started = state.features.start_materialization(request).unwrap();
    let completed = wait_for_materialization(
        &state,
        "alice",
        &started.attempt_id,
        MaterializationAttemptStatus::Completed,
    );
    let dataset_id = completed.dataset_id.unwrap();

    let context = state
        .establish_factor_context("alice", &dataset_id)
        .unwrap();
    let feature_dataset = context.feature_dataset.unwrap();
    assert_eq!(feature_dataset.dataset_id, dataset_id);
    assert_eq!(context.market, "crypto");
    assert_eq!(context.venue, "okx");
    assert_eq!(context.snapshot_id, snapshot_id);
    assert_eq!(context.universe_id.as_deref(), Some(universe_id.as_str()));
    assert_eq!(context.context_revision, 1);

    let candidate = state
        .publish_factor_candidate(crate::factor_research::FactorCandidatePublishRequest {
            user_id: "alice".into(),
            draft: adaq_factor_research::FactorCandidateDraft {
                candidate_id: Uuid::from_u128(169),
                revision: 1,
                scope: adaq_factor_research::FactorScope::CrossSectional,
                feature_slots: vec![adaq_factor_research::FactorFeatureSlot {
                    name: "rank".into(),
                }],
                parameters: vec![],
                outputs: vec![adaq_factor_research::FactorOutput {
                    name: "rank-score".into(),
                }],
                source: adaq_factor_research::FactorCandidateSource::Declarative {
                    definition: adaq_factor_research::DeclarativeFactorDefinition {
                        feature_plan_hash: feature_dataset.feature_plan_hash.clone(),
                        operator_catalog_version:
                            adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION.into(),
                        outputs: vec![adaq_factor_research::DeclarativeFactorOutputBinding {
                            output_name: "rank-score".into(),
                            feature_slot: "rank".into(),
                        }],
                    },
                },
            },
            presentation: adaq_factor_research::FactorPresentationMetadata {
                name: "Rank score".into(),
                description: "published from the Feature handoff".into(),
                tags: vec!["cross-sectional".into()],
            },
        })
        .unwrap();
    let predecessor = candidate.predecessor.as_ref().unwrap();
    assert_eq!(predecessor.user_id, "alice");
    assert_eq!(predecessor.context_revision, context.context_revision);
    assert_eq!(predecessor.context_hash, context.context_hash);
    assert_eq!(predecessor.snapshot_id, snapshot_id);
    assert_eq!(
        predecessor.universe_id.as_deref(),
        Some(universe_id.as_str())
    );
    assert_eq!(predecessor.feature_dataset, feature_dataset);
    assert!(
        state
            .factor
            .get_candidate(crate::factor_research::FactorEvidenceRequest {
                user_id: "bob".into(),
                evidence_id: candidate.candidate.candidate_hash,
            })
            .is_err()
    );

    let frozen = state
        .freeze_research_context(
            "alice",
            "factor-context:one".into(),
            adaq_factor_research::ResearchStage::Factors,
        )
        .unwrap();
    assert_eq!(frozen.feature_dataset.unwrap().dataset_id, dataset_id);
    assert!(
        state
            .require_factor_context_for_request(
                "alice",
                "factor-context:one",
                &dataset_id,
                &feature_dataset.feature_plan_hash,
                &snapshot_id,
                &universe_id,
                Some((0, 3 * HOUR)),
                true,
                "crypto",
                "okx",
                &universe_id,
            )
            .is_ok()
    );
    assert!(
        state
            .require_factor_context_for_request(
                "alice",
                "factor-context:one",
                "another-feature-dataset",
                &feature_dataset.feature_plan_hash,
                &snapshot_id,
                &universe_id,
                Some((0, 3 * HOUR)),
                true,
                "crypto",
                "okx",
                &universe_id,
            )
            .is_err()
    );
    assert!(state.establish_factor_context("bob", &dataset_id).is_err());
    state
        .features
        .delete_dataset(FeatureDatasetRequest {
            user_id: "alice".into(),
            dataset_id: dataset_id.clone(),
        })
        .unwrap();
    assert!(state.research_context_for_user("alice").is_err());
    assert!(
        state
            .require_factor_context_for_request(
                "alice",
                "factor-context:one",
                &dataset_id,
                &feature_dataset.feature_plan_hash,
                &snapshot_id,
                &universe_id,
                Some((0, 3 * HOUR)),
                true,
                "crypto",
                "okx",
                &universe_id,
            )
            .is_err()
    );

    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn materialization_failure_retry_uses_a_new_identity_and_source_evidence() {
    let (root, state, snapshot) = setup("materialization-retry");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    let mut request = materialization_request("alice", &plan, &snapshot.snapshot_id);
    request.snapshot_id = "missing-snapshot".into();
    let started = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "alice".into(),
            request,
            plan: plan_draft(vec![definition]),
        })
        .unwrap();
    let failed = wait_for_materialization(
        &state,
        "alice",
        &started.attempt_id,
        MaterializationAttemptStatus::Failed,
    );
    assert_eq!(
        failed.failure_code.as_deref(),
        Some("feature-evidence-not-found")
    );
    assert_eq!(
        state
            .features
            .retry_materialization_attempt(FeatureAttemptRequest {
                user_id: "bob".into(),
                attempt_id: started.attempt_id.clone(),
            })
            .unwrap_err(),
        "materialization-attempt-not-found"
    );
    let retried = state
        .features
        .retry_materialization_attempt(FeatureAttemptRequest {
            user_id: "alice".into(),
            attempt_id: started.attempt_id.clone(),
        })
        .unwrap();
    assert_ne!(retried.attempt_id, started.attempt_id);
    assert_eq!(
        retried.source_attempt_id.as_deref(),
        Some(started.attempt_id.as_str())
    );
    assert_eq!(retried.request_hash, started.request_hash);
    let failed_again = wait_for_materialization(
        &state,
        "alice",
        &retried.attempt_id,
        MaterializationAttemptStatus::Failed,
    );
    assert_eq!(
        failed_again.failure_code.as_deref(),
        Some("feature-evidence-not-found")
    );
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_device_runs_one_heavy_attempt_at_a_time_in_fifo_and_start_returns_promptly() {
    let (root, state, snapshot) = setup("fifo");
    let hold = RunnerHold::install(&state);
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    let materialization = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "alice".into(),
            request: materialization_request("alice", &plan, &snapshot.snapshot_id),
            plan: plan_draft(vec![definition.clone()]),
        })
        .unwrap();
    hold.wait_for("materialization", &materialization.attempt_id);
    // While the first heavy Attempt is held, a second Start still returns
    // promptly and queues behind it.
    let fitting = state
        .features
        .start_fitting(FeatureFittingStartRequest {
            user_id: "alice".into(),
            protocol: fitting_protocol_draft(&definition, &snapshot.snapshot_id, 2),
            plan: plan_draft(vec![definition]),
        })
        .unwrap();
    assert_eq!(fitting.status, FeatureAttemptStatus::Pending);
    std::thread::sleep(Duration::from_millis(50));
    let still_pending = state
        .features
        .get_fitting_attempt(FeatureAttemptRequest {
            user_id: "alice".into(),
            attempt_id: fitting.attempt_id.clone(),
        })
        .unwrap();
    assert_eq!(
        still_pending.status,
        FeatureAttemptStatus::Pending,
        "the device executes one heavy Feature Attempt at a time"
    );
    hold.release();
    wait_for_materialization(
        &state,
        "alice",
        &materialization.attempt_id,
        MaterializationAttemptStatus::Completed,
    );
    wait_for_fitting(
        &state,
        "alice",
        &fitting.attempt_id,
        FeatureAttemptStatus::Completed,
    );
    let order = hold.started.0.lock().unwrap().clone();
    assert_eq!(order[0].1, materialization.attempt_id);
    assert_eq!(order[1].1, fitting.attempt_id);
    drop(hold);
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_reaches_running_attempts_before_terminal_evidence() {
    let (root, state, snapshot) = setup("cancellation");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    // Cancel every Attempt as soon as it becomes Running; the runner only
    // terminalizes each Attempt after its evaluation loop has stopped.
    let features = state.features.clone();
    *state.features.inner.attempt_started_hook.lock().unwrap() =
        Some(Arc::new(move |kind: &str, attempt_id: &str| {
            if kind == "materialization" {
                features
                    .cancel_materialization_attempt(FeatureAttemptRequest {
                        user_id: "alice".into(),
                        attempt_id: attempt_id.into(),
                    })
                    .unwrap();
            } else {
                features
                    .cancel_fitting_attempt(FeatureAttemptRequest {
                        user_id: "alice".into(),
                        attempt_id: attempt_id.into(),
                    })
                    .unwrap();
            }
        }));
    let materialization = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "alice".into(),
            request: materialization_request("alice", &plan, &snapshot.snapshot_id),
            plan: plan_draft(vec![definition.clone()]),
        })
        .unwrap();
    let fitting = state
        .features
        .start_fitting(FeatureFittingStartRequest {
            user_id: "alice".into(),
            protocol: fitting_protocol_draft(&definition, &snapshot.snapshot_id, 2),
            plan: plan_draft(vec![definition]),
        })
        .unwrap();
    let cancelled_materialization = wait_for_materialization(
        &state,
        "alice",
        &materialization.attempt_id,
        MaterializationAttemptStatus::Cancelled,
    );
    assert!(cancelled_materialization.dataset_id.is_none());
    let cancelled_fitting = wait_for_fitting(
        &state,
        "alice",
        &fitting.attempt_id,
        FeatureAttemptStatus::Cancelled,
    );
    assert!(cancelled_fitting.artifact_id.is_none());
    assert!(
        state
            .features
            .list_datasets(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .list_artifacts(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    let staging = root.join("m3/feature-datasets/staging");
    let leftover = fs::read_dir(&staging)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(leftover, 0, "cancellation must release staging files");
    // A terminal Attempt can no longer be cancelled.
    assert_eq!(
        state
            .features
            .cancel_materialization_attempt(FeatureAttemptRequest {
                user_id: "alice".into(),
                attempt_id: materialization.attempt_id,
            })
            .unwrap_err(),
        "Feature Materialization Attempt cannot be cancelled"
    );
    *state.features.inner.attempt_started_hook.lock().unwrap() = None;
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_attempts_survive_restart_and_running_recovers_to_failed() {
    let (root, state, snapshot) = setup("recovery");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    let protocol = TransformationFittingProtocol::freeze(TransformationFittingProtocolDraft {
        engine_identity: FeatureEngineIdentity::native().unwrap(),
        ..fitting_protocol_draft(&definition, &snapshot.snapshot_id, 2)
    })
    .unwrap();
    let pending_fitting_id = "seeded-pending-fitting";
    let running_fitting_id = "seeded-running-fitting";
    let protocol_json = String::from_utf8(protocol.to_json()).unwrap();
    let plan_json = String::from_utf8(plan.to_json()).unwrap();
    let request = materialization_request("alice", &plan, &snapshot.snapshot_id)
        .with_plan_evidence(&plan)
        .unwrap();
    let request_json = serde_json::to_string(&request).unwrap();
    let request_hash = request.request_hash();
    let engine_identity_json = serde_json::to_string(&plan.engine_identity()).unwrap();
    drop(state);

    // Seed attempts while the application is closed: Pending survives,
    // Running becomes interruption evidence on the next open.
    let database = rusqlite::Connection::open(root.join("adaq.db")).unwrap();
    let now = crate::features::store::now_ms();
    database
        .execute(
            "INSERT INTO feature_fitting_protocols(protocol_hash, protocol_json, created_at_ms)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![protocol.protocol_hash(), protocol_json, now],
        )
        .unwrap();
    for (attempt_id, status) in [
        (pending_fitting_id, "pending"),
        (running_fitting_id, "running"),
    ] {
        database
            .execute(
                "INSERT INTO feature_fitting_attempts(
                     attempt_id, user_id, protocol_hash, plan_hash, plan_json, status,
                     progress_completed, progress_total, created_at_ms, updated_at_ms
                 ) VALUES (?1, 'alice', ?2, ?3, ?4, ?5, 0, 0, ?6, ?6)",
                rusqlite::params![
                    attempt_id,
                    protocol.protocol_hash(),
                    plan.plan_hash(),
                    plan_json,
                    status,
                    now
                ],
            )
            .unwrap();
    }
    database
        .execute(
            "INSERT INTO feature_materialization_requests(
                 request_hash, user_id, request_json, plan_json, artifact_ids_json,
                 engine_identity_json, output_names_json, created_at_ms
             ) VALUES (?1, 'alice', ?2, ?3, '[]', ?4, '[\"return\"]', ?5)",
            rusqlite::params![
                request_hash,
                request_json,
                plan_json,
                engine_identity_json,
                now
            ],
        )
        .unwrap();
    for (attempt_id, status) in [
        ("seeded-pending-materialization", "pending"),
        ("seeded-running-materialization", "running"),
    ] {
        database
            .execute(
                "INSERT INTO feature_materialization_attempts(
                     attempt_id, request_hash, user_id, status,
                     progress_completed, progress_total, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 'alice', ?3, 0, 0, ?4, ?4)",
                rusqlite::params![attempt_id, request_hash, status, now],
            )
            .unwrap();
    }
    drop(database);

    let state = LocalResearchState::open(&root).unwrap();
    let recovered_running_fitting = wait_for_fitting(
        &state,
        "alice",
        running_fitting_id,
        FeatureAttemptStatus::Failed,
    );
    assert_eq!(
        recovered_running_fitting.failure_code.as_deref(),
        Some("interrupted")
    );
    assert!(
        recovered_running_fitting
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("interrupted")
    );
    wait_for_fitting(
        &state,
        "alice",
        pending_fitting_id,
        FeatureAttemptStatus::Completed,
    );
    let recovered_running_materialization = wait_for_materialization(
        &state,
        "alice",
        "seeded-running-materialization",
        MaterializationAttemptStatus::Failed,
    );
    assert_eq!(
        recovered_running_materialization.failure_code.as_deref(),
        Some("interrupted")
    );
    let completed_materialization = wait_for_materialization(
        &state,
        "alice",
        "seeded-pending-materialization",
        MaterializationAttemptStatus::Completed,
    );
    assert!(completed_materialization.dataset_id.is_some());
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deletion_checks_references_and_dedup_grants_no_cross_user_visibility() {
    let (root, state, snapshot) = setup("deletion");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    let alice_attempt = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "alice".into(),
            request: materialization_request("alice", &plan, &snapshot.snapshot_id),
            plan: plan_draft(vec![definition.clone()]),
        })
        .unwrap();
    let alice_completed = wait_for_materialization(
        &state,
        "alice",
        &alice_attempt.attempt_id,
        MaterializationAttemptStatus::Completed,
    );
    let alice_dataset = alice_completed.dataset_id.unwrap();
    // Bob materializes the same effective evidence; device bytes are
    // deduplicated but his visibility stays his own.
    state
        .grant_snapshot_for_user("bob", &snapshot.snapshot_id)
        .unwrap();
    let bob_attempt = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "bob".into(),
            request: materialization_request("bob", &plan, &snapshot.snapshot_id),
            plan: plan_draft(vec![definition]),
        })
        .unwrap();
    let bob_completed = wait_for_materialization(
        &state,
        "bob",
        &bob_attempt.attempt_id,
        MaterializationAttemptStatus::Completed,
    );
    let bob_dataset = bob_completed.dataset_id.unwrap();
    assert_ne!(alice_dataset, bob_dataset);
    let alice_parquet = state
        .features
        .get_dataset(FeatureDatasetRequest {
            user_id: "alice".into(),
            dataset_id: alice_dataset.clone(),
        })
        .unwrap();
    let bob_parquet = state
        .features
        .get_dataset(FeatureDatasetRequest {
            user_id: "bob".into(),
            dataset_id: bob_dataset.clone(),
        })
        .unwrap();
    assert_eq!(
        alice_parquet.manifest.content_sha256, bob_parquet.manifest.content_sha256,
        "identical evidence deduplicates device bytes"
    );
    // Alice deleting her Dataset must not remove Bob's bytes or visibility.
    state
        .features
        .delete_dataset(FeatureDatasetRequest {
            user_id: "alice".into(),
            dataset_id: alice_dataset.clone(),
        })
        .unwrap();
    assert!(
        state
            .features
            .get_dataset(FeatureDatasetRequest {
                user_id: "alice".into(),
                dataset_id: alice_dataset,
            })
            .is_err()
    );
    assert!(
        state
            .features
            .get_dataset(FeatureDatasetRequest {
                user_id: "bob".into(),
                dataset_id: bob_dataset.clone(),
            })
            .is_ok(),
        "deduplication grants no cross-User visibility changes"
    );
    // A downstream local research reference locks Bob's Dataset.
    state
        .features
        .inner
        .materialization
        .reference_dataset("bob", &bob_dataset, "factor-research", "factor-run-1")
        .unwrap();
    assert_eq!(
        state
            .features
            .delete_dataset(FeatureDatasetRequest {
                user_id: "bob".into(),
                dataset_id: bob_dataset.clone(),
            })
            .unwrap_err(),
        "feature-dataset-referenced"
    );
    state
        .features
        .inner
        .materialization
        .unreference_dataset("factor-research", &bob_dataset, "factor-run-1")
        .unwrap();
    state
        .features
        .delete_dataset(FeatureDatasetRequest {
            user_id: "bob".into(),
            dataset_id: bob_dataset,
        })
        .unwrap();
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reset_all_fails_before_deletion_when_an_attempt_cannot_stop() {
    let (root, state, snapshot) = setup("reset-stuck");
    let watchlist = WatchlistDb::open(&root.join("adaq.db")).unwrap();
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    let hold = RunnerHold::install(&state);
    let started = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "alice".into(),
            request: materialization_request("alice", &plan, &snapshot.snapshot_id),
            plan: plan_draft(vec![definition]),
        })
        .unwrap();
    hold.wait_for("materialization", &started.attempt_id);
    state
        .features
        .set_reset_wait_timeout(Duration::from_millis(100));
    let error = state
        .reset_local_data("alice", crate::local_research::LocalDataResetKind::All)
        .unwrap_err();
    assert!(error.contains("could not stop Feature work"), "{error}");
    assert!(
        state.features.inner.reset_blocks.lock().unwrap().is_empty(),
        "the start restriction must be released on failure"
    );
    hold.release();
    wait_for_materialization(
        &state,
        "alice",
        &started.attempt_id,
        MaterializationAttemptStatus::Cancelled,
    );
    drop(hold);
    drop(watchlist);
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn plan_freeze_preview_uses_the_native_identity_and_creates_no_evidence() {
    let (root, state, _snapshot) = setup("plan-freeze-preview");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    // The GUI submits camelCase JSON with an empty engine identity; this
    // deserialization proves the wire shape, and the module replaces the
    // identity with the native one before freezing.
    let gui_json = serde_json::json!({
        "userId": "alice",
        "plan": {
            "definitions": [serde_json::to_value(&definition).unwrap()],
            "slots": [],
            "factors": [],
            "artifacts": [],
            "consumerPackageSha256": "",
            "consumerParameters": [],
            "consumerWarmupBars": 0,
            "engineIdentity": {
                "featureEngineVersion": "",
                "featureEngineSourceSha256": "",
                "featureEngineBuildId": "",
                "operatorCatalogVersion": "",
                "indicatorEngineVersion": "",
                "indicatorCatalogVersion": "",
                "taLibVersion": "",
                "taSourceSha256": "",
                "wrapperSha256": "",
                "targetTriple": "",
                "compilerAndFlagsSha256": "",
                "engineBuildId": ""
            }
        }
    });
    let gui_request: FeaturePlanDraftRequest = serde_json::from_value(gui_json).unwrap();
    let view = state.features.freeze_plan_for_user(gui_request).unwrap();
    assert_eq!(view.plan_hash, native_plan(vec![definition]).plan_hash());
    assert_eq!(
        FeaturePlan::load(view.plan_json.as_bytes())
            .unwrap()
            .plan_hash(),
        view.plan_hash
    );
    assert!(
        state
            .features
            .list_fitting_attempts(FeatureUserRequest {
                user_id: "alice".into(),
            })
            .unwrap()
            .is_empty()
    );
    let invalid = state
        .features
        .freeze_plan_for_user(FeaturePlanDraftRequest {
            user_id: "alice".into(),
            plan: FeaturePlanDraft::default(),
        });
    assert!(
        invalid
            .unwrap_err()
            .starts_with("feature-plan-validation-failed")
    );
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn artifact_deletion_is_locked_by_typed_references() {
    let (root, state, snapshot) = setup("artifact-lock");
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let completed = state
        .features
        .start_fitting(FeatureFittingStartRequest {
            user_id: "alice".into(),
            protocol: fitting_protocol_draft(&definition, &snapshot.snapshot_id, 2),
            plan: plan_draft(vec![definition]),
        })
        .unwrap();
    let completed = wait_for_fitting(
        &state,
        "alice",
        &completed.attempt_id,
        FeatureAttemptStatus::Completed,
    );
    let artifact_id = completed.artifact_id.unwrap();
    let database = state.database.lock().unwrap();
    database
        .execute(
            "INSERT INTO feature_artifact_references(artifact_id, referencing_user_id, reference_id)
             VALUES (?1, 'factor-research', 'factor-plan-1')",
            [&artifact_id],
        )
        .unwrap();
    drop(database);
    assert_eq!(
        state
            .features
            .delete_artifact(FeatureArtifactRequest {
                user_id: "alice".into(),
                artifact_id: artifact_id.clone(),
            })
            .unwrap_err(),
        "artifact-referenced"
    );
    let database = state.database.lock().unwrap();
    database
        .execute(
            "DELETE FROM feature_artifact_references WHERE artifact_id = ?1",
            [&artifact_id],
        )
        .unwrap();
    drop(database);
    state
        .features
        .delete_artifact(FeatureArtifactRequest {
            user_id: "alice".into(),
            artifact_id: artifact_id.clone(),
        })
        .unwrap();
    assert!(
        state
            .features
            .get_artifact(FeatureArtifactRequest {
                user_id: "alice".into(),
                artifact_id,
            })
            .is_err()
    );
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reset_all_stops_feature_work_and_deletes_only_that_users_evidence() {
    let (root, state, snapshot) = setup("reset");
    let watchlist = WatchlistDb::open(&root.join("adaq.db")).unwrap();
    let definition = FeatureDefinition::freeze(return_draft()).unwrap();
    let plan = native_plan(vec![definition.clone()]);
    state
        .features
        .publish_definition(DefinitionPublishRequest {
            user_id: "alice".into(),
            draft: return_draft(),
            name: "Return".into(),
            description: String::new(),
            tags: vec![],
        })
        .unwrap();
    let fitting = state
        .features
        .start_fitting(FeatureFittingStartRequest {
            user_id: "alice".into(),
            protocol: fitting_protocol_draft(&definition, &snapshot.snapshot_id, 2),
            plan: plan_draft(vec![definition.clone()]),
        })
        .unwrap();
    wait_for_fitting(
        &state,
        "alice",
        &fitting.attempt_id,
        FeatureAttemptStatus::Completed,
    );
    let materialization = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "alice".into(),
            request: materialization_request("alice", &plan, &snapshot.snapshot_id),
            plan: plan_draft(vec![definition.clone()]),
        })
        .unwrap();
    wait_for_materialization(
        &state,
        "alice",
        &materialization.attempt_id,
        MaterializationAttemptStatus::Completed,
    );
    // Bob keeps independent evidence on the same Snapshot.
    state
        .grant_snapshot_for_user("bob", &snapshot.snapshot_id)
        .unwrap();
    let bob_materialization = state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: "bob".into(),
            request: materialization_request("bob", &plan, &snapshot.snapshot_id),
            plan: plan_draft(vec![definition]),
        })
        .unwrap();
    wait_for_materialization(
        &state,
        "bob",
        &bob_materialization.attempt_id,
        MaterializationAttemptStatus::Completed,
    );

    state
        .reset_local_data("alice", crate::local_research::LocalDataResetKind::All)
        .unwrap();

    assert!(
        state
            .features
            .list_definitions(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .list_fitting_attempts(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .list_materialization_attempts(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .list_datasets(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .features
            .list_artifacts(FeatureUserRequest {
                user_id: "alice".into()
            })
            .unwrap()
            .is_empty()
    );
    let bob_attempts_after = state
        .features
        .list_materialization_attempts(FeatureUserRequest {
            user_id: "bob".into(),
        })
        .unwrap();
    assert_eq!(
        bob_attempts_after.len(),
        1,
        "bob's evidence survives alice's Reset All"
    );
    assert!(
        state
            .features
            .get_dataset(FeatureDatasetRequest {
                user_id: "bob".into(),
                dataset_id: bob_attempts_after[0]
                    .dataset_id
                    .clone()
                    .expect("bob's completed Attempt keeps its Dataset"),
            })
            .is_ok()
    );
    drop(watchlist);
    drop(state);
    fs::remove_dir_all(root).unwrap();
}
