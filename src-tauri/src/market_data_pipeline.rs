//! Thin Tauri adapters for the Tauri-independent market-data pipeline.

use std::collections::BTreeMap;

use adaq_backtest_core::MarketDataSnapshot;
use adaq_data_core::BarInterval;
use adaq_data_core::market::InstrumentId;
use adaq_data_pipeline::{
    AcquisitionDiagnostics, CanonicalWarning, CanonicalizationRequest, DataQualityReport,
    DataQualityState, PipelinePublication, ProviderCapabilitySnapshot, QuarantineReason,
    SourceAcquisition, SourceMarketRecord,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishRequest {
    pub task_id: String,
    pub user_id: String,
    pub acquisition: AcquisitionRequest,
    pub canonicalization: CanonicalizationRequest,
}

impl PublishRequest {
    pub(crate) fn into_parts(
        self,
    ) -> Result<(String, String, SourceAcquisition, CanonicalizationRequest), String> {
        Ok((
            self.task_id,
            self.user_id,
            self.acquisition.into_acquisition()?,
            self.canonicalization,
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcquisitionRequest {
    pub provider: String,
    pub actual_upstream: Option<String>,
    pub connector: String,
    pub connector_version: String,
    #[serde(default)]
    pub request_parameters: BTreeMap<String, String>,
    pub retrieved_at_ms: i64,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    #[serde(default)]
    pub acquisition_diagnostics: AcquisitionDiagnostics,
    pub records: Vec<SourceRecordRequest>,
}

impl AcquisitionRequest {
    fn into_acquisition(self) -> Result<SourceAcquisition, String> {
        let request_parameters = self
            .request_parameters
            .into_iter()
            .map(|(key, value)| (key, serde_json::Value::String(value)))
            .collect();
        Ok(SourceAcquisition {
            provider: self.provider,
            actual_upstream: self.actual_upstream,
            connector: self.connector,
            connector_version: self.connector_version,
            request_parameters: serde_json::Value::Object(request_parameters),
            retrieved_at_ms: self.retrieved_at_ms,
            capability_snapshot: self.capability_snapshot,
            acquisition_diagnostics: self.acquisition_diagnostics,
            records: self
                .records
                .into_iter()
                .map(SourceRecordRequest::into_record)
                .collect(),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceRecordRequest {
    pub provider_symbol: String,
    pub instrument: InstrumentId,
    pub interval: BarInterval,
    pub open_time_ms: i64,
    pub open: Option<String>,
    pub high: Option<String>,
    pub low: Option<String>,
    pub close: Option<String>,
    pub base_volume: Option<String>,
    pub quote_volume: Option<String>,
}

impl SourceRecordRequest {
    fn into_record(self) -> SourceMarketRecord {
        SourceMarketRecord {
            provider_symbol: self.provider_symbol,
            instrument: self.instrument,
            interval: self.interval,
            open_time_ms: self.open_time_ms,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            base_volume: self.base_volume,
            quote_volume: self.quote_volume,
            raw_payload: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserEvidenceRequest {
    pub user_id: String,
    pub evidence_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserRequest {
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UniverseRequest {
    pub user_id: String,
    pub as_of_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackfillCancelRequest {
    pub user_id: String,
    pub task_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotRequest {
    pub user_id: String,
    pub canonical_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotPublicationView {
    pub snapshot: MarketDataSnapshot,
    pub quality: QualityView,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteRequest {
    pub user_id: String,
    pub evidence_kind: String,
    pub evidence_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicationView {
    pub attempt_id: Option<String>,
    pub source_id: String,
    pub source_revision: u64,
    pub canonical_id: Option<String>,
    pub report_id: String,
    pub state: DataQualityState,
    pub source_record_count: usize,
    pub canonical_record_count: usize,
    pub quarantine_count: usize,
    pub conflict_count: usize,
    pub duplicate_count: usize,
    pub gap_count: usize,
    pub warning_count: usize,
}

impl From<PipelinePublication> for PublicationView {
    fn from(value: PipelinePublication) -> Self {
        Self {
            attempt_id: value.attempt_id,
            source_id: value.source.source_id,
            source_revision: value.source.revision,
            canonical_id: value
                .canonical
                .as_ref()
                .map(|dataset| dataset.canonical_id.clone()),
            report_id: value.quality.report_id,
            state: value.quality.state,
            source_record_count: value.source.records.len(),
            canonical_record_count: value.quality.coverage.canonical_record_count,
            quarantine_count: value.quality.quarantine_count,
            conflict_count: value.quality.conflict_count,
            duplicate_count: value.quality.duplicate_count,
            gap_count: value.quality.gap_count,
            warning_count: value.quality.warning_count,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualityView {
    pub report_id: String,
    pub source_id: String,
    pub canonical_id: Option<String>,
    pub state: DataQualityState,
    pub applied_rules: Vec<String>,
    pub coverage: adaq_data_pipeline::Coverage,
    pub duplicate_count: usize,
    pub conflict_count: usize,
    pub quarantine_count: usize,
    pub gap_count: usize,
    pub warning_count: usize,
    pub capability_limitations: Vec<String>,
    pub reasons: Vec<adaq_data_pipeline::QualityReason>,
    pub quarantined: Vec<QuarantinedView>,
    pub warnings: Vec<CanonicalWarning>,
    pub gaps: Vec<adaq_data_core::BarGap>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuarantinedView {
    pub source_record_hash: String,
    pub reason: QuarantineReason,
}

impl From<DataQualityReport> for QualityView {
    fn from(value: DataQualityReport) -> Self {
        Self {
            report_id: value.report_id,
            source_id: value.source_id,
            canonical_id: value.canonical_id,
            state: value.state,
            applied_rules: value.applied_rules,
            coverage: value.coverage,
            duplicate_count: value.duplicate_count,
            conflict_count: value.conflict_count,
            quarantine_count: value.quarantine_count,
            gap_count: value.gap_count,
            warning_count: value.warning_count,
            capability_limitations: value.capability_limitations,
            reasons: value.reasons,
            quarantined: value
                .quarantined_records
                .into_iter()
                .map(|record| QuarantinedView {
                    source_record_hash: record.source_record_hash,
                    reason: record.reason,
                })
                .collect(),
            warnings: value.warnings,
            gaps: value.gaps,
        }
    }
}
