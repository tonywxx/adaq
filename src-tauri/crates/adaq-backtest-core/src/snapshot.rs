use std::{
    fmt::Write as _,
    fs::{self, File},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use adaq_data_core::{BarGap, BarInterval, BarSeries, OhlcvBar};
use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataSnapshot {
    pub snapshot_id: String,
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub bar_count: usize,
    pub gaps: Vec<SnapshotGap>,
    pub parquet_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotGap {
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

impl From<BarGap> for SnapshotGap {
    fn from(value: BarGap) -> Self {
        Self {
            start_time_ms: value.start_time_ms,
            end_time_ms: value.end_time_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotError(pub String);

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SnapshotError {}

pub struct SnapshotStore {
    root: PathBuf,
}

impl SnapshotStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, SnapshotError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(error)?;
        Ok(Self { root })
    }

    pub fn persist(&self, series: &BarSeries) -> Result<MarketDataSnapshot, SnapshotError> {
        validate_series(series)?;
        let snapshot_id = content_hash(series);
        let parquet_path = self.root.join(format!("{snapshot_id}.parquet"));
        if !parquet_path.is_file() {
            write_parquet(&parquet_path, &series.bars)?;
        }
        Ok(MarketDataSnapshot {
            snapshot_id,
            src: series.src.clone(),
            code: series.code.clone(),
            interval: series.interval,
            start_time_ms: series.bars.first().map_or(0, |bar| bar.open_time_ms),
            end_time_ms: series.bars.last().map_or(0, |bar| bar.open_time_ms),
            bar_count: series.bars.len(),
            gaps: series.gaps.iter().copied().map(Into::into).collect(),
            parquet_path,
        })
    }

    pub fn read(&self, snapshot: &MarketDataSnapshot) -> Result<Vec<OhlcvBar>, SnapshotError> {
        if !snapshot.parquet_path.starts_with(&self.root) {
            return Err(SnapshotError(
                "Snapshot path is outside the data store".into(),
            ));
        }
        read_parquet(&snapshot.parquet_path)
    }
}

fn validate_series(series: &BarSeries) -> Result<(), SnapshotError> {
    if series.src.trim().is_empty() || series.code.trim().is_empty() || series.bars.is_empty() {
        return Err(SnapshotError("Market Data Snapshot cannot be empty".into()));
    }
    if series
        .bars
        .windows(2)
        .any(|bars| bars[0].open_time_ms >= bars[1].open_time_ms)
    {
        return Err(SnapshotError(
            "Closed Bars must be strictly ascending".into(),
        ));
    }
    Ok(())
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("open_time_ms", DataType::Int64, false),
        Field::new("open", DataType::Utf8, false),
        Field::new("high", DataType::Utf8, false),
        Field::new("low", DataType::Utf8, false),
        Field::new("close", DataType::Utf8, false),
        Field::new("base_volume", DataType::Utf8, false),
        Field::new("quote_volume", DataType::Utf8, false),
    ]))
}

fn write_parquet(path: &Path, bars: &[OhlcvBar]) -> Result<(), SnapshotError> {
    let batch = RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(Int64Array::from_iter_values(
                bars.iter().map(|bar| bar.open_time_ms),
            )),
            string_column(bars, |bar| bar.open),
            string_column(bars, |bar| bar.high),
            string_column(bars, |bar| bar.low),
            string_column(bars, |bar| bar.close),
            string_column(bars, |bar| bar.base_volume),
            string_column(bars, |bar| bar.quote_volume),
        ],
    )
    .map_err(error)?;
    let temporary = path.with_extension("parquet.tmp");
    let file = File::create(&temporary).map_err(error)?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), None).map_err(error)?;
    writer.write(&batch).map_err(error)?;
    writer.close().map_err(error)?;
    fs::rename(&temporary, path).map_err(error)
}

fn string_column(bars: &[OhlcvBar], value: impl Fn(&OhlcvBar) -> Decimal) -> Arc<StringArray> {
    Arc::new(StringArray::from_iter_values(
        bars.iter().map(|bar| value(bar).to_string()),
    ))
}

fn read_parquet(path: &Path) -> Result<Vec<OhlcvBar>, SnapshotError> {
    let file = File::open(path).map_err(error)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(error)?
        .with_batch_size(8192)
        .build()
        .map_err(error)?;
    let mut bars = Vec::new();
    for batch in reader {
        let batch = batch.map_err(error)?;
        let times = column::<Int64Array>(&batch, 0)?;
        let open = column::<StringArray>(&batch, 1)?;
        let high = column::<StringArray>(&batch, 2)?;
        let low = column::<StringArray>(&batch, 3)?;
        let close = column::<StringArray>(&batch, 4)?;
        let base_volume = column::<StringArray>(&batch, 5)?;
        let quote_volume = column::<StringArray>(&batch, 6)?;
        for index in 0..batch.num_rows() {
            bars.push(OhlcvBar {
                open_time_ms: times.value(index),
                open: decimal(open.value(index))?,
                high: decimal(high.value(index))?,
                low: decimal(low.value(index))?,
                close: decimal(close.value(index))?,
                base_volume: decimal(base_volume.value(index))?,
                quote_volume: decimal(quote_volume.value(index))?,
            });
        }
    }
    Ok(bars)
}

fn column<T: Array + 'static>(batch: &RecordBatch, index: usize) -> Result<&T, SnapshotError> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| SnapshotError("Snapshot Parquet schema is invalid".into()))
}

fn decimal(value: &str) -> Result<Decimal, SnapshotError> {
    Decimal::from_str(value).map_err(error)
}

fn content_hash(series: &BarSeries) -> String {
    let mut hasher = Sha256::new();
    hasher.update(series.src.as_bytes());
    hasher.update([0]);
    hasher.update(series.code.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(&series.interval).expect("BarInterval serializes"));
    for bar in &series.bars {
        hasher.update(bar.open_time_ms.to_le_bytes());
        for value in [
            bar.open,
            bar.high,
            bar.low,
            bar.close,
            bar.base_volume,
            bar.quote_volume,
        ] {
            hasher.update(value.to_string().as_bytes());
            hasher.update([0]);
        }
    }
    for gap in &series.gaps {
        hasher.update(gap.start_time_ms.to_le_bytes());
        hasher.update(gap.end_time_ms.to_le_bytes());
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn error(error: impl std::fmt::Display) -> SnapshotError {
    SnapshotError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adaq_data_core::BarSeries;

    fn bar(time: i64, close: i64) -> OhlcvBar {
        let value = Decimal::from(close);
        OhlcvBar {
            open_time_ms: time,
            open: value,
            high: value,
            low: value,
            close: value,
            base_volume: Decimal::ONE,
            quote_volume: value,
        }
    }

    #[test]
    fn parquet_snapshot_round_trips_and_deduplicates() {
        let directory = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(directory.path()).unwrap();
        let series = BarSeries {
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneMinute,
            bars: vec![bar(1, 10), bar(2, 11)],
            gaps: vec![BarGap {
                start_time_ms: 3,
                end_time_ms: 4,
            }],
        };
        let first = store.persist(&series).unwrap();
        let second = store.persist(&series).unwrap();
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(store.read(&first).unwrap(), series.bars);
        assert_eq!(
            first.gaps,
            vec![SnapshotGap {
                start_time_ms: 3,
                end_time_ms: 4
            }]
        );

        let mut changed = series.clone();
        changed.bars[1].close = Decimal::from(12);
        assert_ne!(
            store.persist(&changed).unwrap().snapshot_id,
            first.snapshot_id
        );
    }
}
