//! Parquet writer / reader for the unified `sessions.parquet` table.
//!
//! Strict schema validation: every reader checks the column names and types
//! against the documented layout and returns [`Error::SchemaMismatch`] on any
//! divergence. The reader never panics, never silently coerces, and never
//! tolerates a partial column set.
//!
//! # File layout
//!
//! ```text
//! data/
//! └── sessions.parquet
//! ```
//!
//! # Schema
//!
//! ```text
//! root                       : Utf8                 (non-null)
//! trading_class              : Utf8                 (non-null)
//! regular_open_us            : Int64                (non-null)
//! regular_close_us           : Int64                (non-null)
//! pre_open_us                : Int64                (nullable)
//! pre_close_us               : Int64                (nullable)
//! post_open_us               : Int64                (nullable)
//! post_close_us              : Int64                (nullable)
//! curb_open_us               : Int64                (nullable)
//! curb_close_us              : Int64                (nullable)
//! gth_open_us                : Int64                (nullable)
//! gth_close_us               : Int64                (nullable)
//! gth_overnight              : Boolean              (nullable)
//! last_trading_day_close_us  : Int64                (nullable)
//! settlement_am_open_us      : Int64                (nullable)
//! ```
//!
//! `settlement_am_open_us` encodes the AM SET print microsecond-of-day
//! for [`Settlement::AmOpen`][crate::Settlement::AmOpen] rows
//! (currently the SPX root family at 09:30 ET); NULL maps to
//! [`Settlement::Pm`][crate::Settlement::Pm].
//!
//! Compression: ZSTD level 3. Row group size: 10 000 rows.

use arrow::array::{Array, BooleanArray, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::session::{SessionInfo, Settlement, TimeWindow, TradingClass};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ROW_GROUP_SIZE: usize = 10_000;

/// File name shipped under `data/`.
pub const FILE_SESSIONS: &str = "sessions.parquet";

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

fn sessions_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("root", DataType::Utf8, false),
        Field::new("trading_class", DataType::Utf8, false),
        Field::new("regular_open_us", DataType::Int64, false),
        Field::new("regular_close_us", DataType::Int64, false),
        Field::new("pre_open_us", DataType::Int64, true),
        Field::new("pre_close_us", DataType::Int64, true),
        Field::new("post_open_us", DataType::Int64, true),
        Field::new("post_close_us", DataType::Int64, true),
        Field::new("curb_open_us", DataType::Int64, true),
        Field::new("curb_close_us", DataType::Int64, true),
        Field::new("gth_open_us", DataType::Int64, true),
        Field::new("gth_close_us", DataType::Int64, true),
        Field::new("gth_overnight", DataType::Boolean, true),
        Field::new("last_trading_day_close_us", DataType::Int64, true),
        Field::new("settlement_am_open_us", DataType::Int64, true),
    ]))
}

fn writer_props() -> Result<WriterProperties> {
    let level =
        ZstdLevel::try_new(3).map_err(|e| Error::Parquet(format!("invalid zstd level: {e}")))?;
    Ok(WriterProperties::builder()
        .set_compression(Compression::ZSTD(level))
        .set_max_row_group_size(ROW_GROUP_SIZE)
        .build())
}

// ---------------------------------------------------------------------------
// Schema validation helpers
// ---------------------------------------------------------------------------

fn render_schema(schema: &Schema) -> String {
    schema
        .fields()
        .iter()
        .map(|f| {
            format!(
                "{}: {:?} (nullable={})",
                f.name(),
                f.data_type(),
                f.is_nullable()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn check_schema(file: &str, expected: &Schema, actual: &Schema) -> Result<()> {
    if expected.fields().len() != actual.fields().len() {
        return Err(Error::SchemaMismatch {
            file: file.to_string(),
            expected: render_schema(expected),
            found: render_schema(actual),
        });
    }
    for (e, a) in expected.fields().iter().zip(actual.fields().iter()) {
        if e.name() != a.name()
            || e.data_type() != a.data_type()
            || e.is_nullable() != a.is_nullable()
        {
            return Err(Error::SchemaMismatch {
                file: file.to_string(),
                expected: render_schema(expected),
                found: render_schema(actual),
            });
        }
    }
    Ok(())
}

fn cast_string_col<'a>(
    batch: &'a RecordBatch,
    file: &str,
    col: usize,
    name: &str,
) -> Result<&'a StringArray> {
    batch
        .column(col)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| Error::SchemaMismatch {
            file: file.to_string(),
            expected: format!("{name}: Utf8"),
            found: format!("{name}: {:?}", batch.column(col).data_type()),
        })
}

fn cast_i64_col<'a>(
    batch: &'a RecordBatch,
    file: &str,
    col: usize,
    name: &str,
) -> Result<&'a Int64Array> {
    batch
        .column(col)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| Error::SchemaMismatch {
            file: file.to_string(),
            expected: format!("{name}: Int64"),
            found: format!("{name}: {:?}", batch.column(col).data_type()),
        })
}

fn cast_bool_col<'a>(
    batch: &'a RecordBatch,
    file: &str,
    col: usize,
    name: &str,
) -> Result<&'a BooleanArray> {
    batch
        .column(col)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .ok_or_else(|| Error::SchemaMismatch {
            file: file.to_string(),
            expected: format!("{name}: Boolean"),
            found: format!("{name}: {:?}", batch.column(col).data_type()),
        })
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Write the unified session table to `data_dir/sessions.parquet`.
///
/// Rows are normalised to upper-case roots and sorted ascending. Duplicate
/// `(root, trading_class)` pairs are NOT deduplicated by this routine — the
/// seed-data layer owns that decision because two distinct trading classes
/// can legitimately share a root (e.g. SPY equity vs. SPY options on
/// different exchanges).
///
/// # Errors
///
/// Returns [`Error`] if the file cannot be created or the parquet writer
/// fails (Arrow / parquet errors are wrapped).
pub fn write_sessions(data_dir: &Path, rows: &[SessionInfo]) -> Result<()> {
    fs::create_dir_all(data_dir)?;

    let mut sorted: Vec<SessionInfo> = rows
        .iter()
        .cloned()
        .map(|mut r| {
            r.root = r.root.to_ascii_uppercase();
            r
        })
        .collect();
    sorted.sort_by(|a, b| {
        a.root
            .cmp(&b.root)
            .then_with(|| a.trading_class.as_wire().cmp(&b.trading_class.as_wire()))
    });

    let roots: StringArray = sorted.iter().map(|r| Some(r.root.as_str())).collect();
    let classes: StringArray = sorted
        .iter()
        .map(|r| Some(r.trading_class.as_wire()))
        .collect();
    let regular_open: Int64Array = sorted.iter().map(|r| Some(r.regular.open_us())).collect();
    let regular_close: Int64Array = sorted.iter().map(|r| Some(r.regular.close_us())).collect();
    let pre_open: Int64Array = sorted
        .iter()
        .map(|r| r.pre_market.as_ref().map(|w| w.open_us()))
        .collect();
    let pre_close: Int64Array = sorted
        .iter()
        .map(|r| r.pre_market.as_ref().map(|w| w.close_us()))
        .collect();
    let post_open: Int64Array = sorted
        .iter()
        .map(|r| r.post_market.as_ref().map(|w| w.open_us()))
        .collect();
    let post_close: Int64Array = sorted
        .iter()
        .map(|r| r.post_market.as_ref().map(|w| w.close_us()))
        .collect();
    let curb_open: Int64Array = sorted
        .iter()
        .map(|r| r.curb.as_ref().map(|w| w.open_us()))
        .collect();
    let curb_close: Int64Array = sorted
        .iter()
        .map(|r| r.curb.as_ref().map(|w| w.close_us()))
        .collect();
    let gth_open: Int64Array = sorted
        .iter()
        .map(|r| r.gth.as_ref().map(|w| w.open_us()))
        .collect();
    let gth_close: Int64Array = sorted
        .iter()
        .map(|r| r.gth.as_ref().map(|w| w.close_us()))
        .collect();
    let gth_overnight: BooleanArray = sorted
        .iter()
        .map(|r| r.gth.as_ref().map(|_| r.gth_overnight))
        .collect();
    let last_trading_day_close: Int64Array =
        sorted.iter().map(|r| r.last_trading_day_close_us).collect();
    let settlement_am_open: Int64Array = sorted
        .iter()
        .map(|r| match r.settlement {
            Settlement::AmOpen { open_us_of_day } => Some(open_us_of_day),
            Settlement::Pm => None,
        })
        .collect();

    let schema = sessions_schema();
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(roots),
            Arc::new(classes),
            Arc::new(regular_open),
            Arc::new(regular_close),
            Arc::new(pre_open),
            Arc::new(pre_close),
            Arc::new(post_open),
            Arc::new(post_close),
            Arc::new(curb_open),
            Arc::new(curb_close),
            Arc::new(gth_open),
            Arc::new(gth_close),
            Arc::new(gth_overnight),
            Arc::new(last_trading_day_close),
            Arc::new(settlement_am_open),
        ],
    )?;

    let path = data_dir.join(FILE_SESSIONS);
    let file = fs::File::create(&path)?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(writer_props()?))?;
    writer.write(&batch)?;
    writer.close()?;

    tracing::info!("wrote {} session rows -> {}", sorted.len(), path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Read `sessions.parquet` from `path` into a sorted vec.
///
/// # Errors
///
/// Returns [`Error::SchemaMismatch`] if the file's schema does not match the
/// documented layout. Returns [`Error`] on file-open or parquet-decode
/// failure.
pub fn read_sessions(path: &Path) -> Result<Vec<SessionInfo>> {
    let file = fs::File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let actual_schema = Arc::clone(builder.schema());
    let expected = sessions_schema();
    check_schema(FILE_SESSIONS, &expected, &actual_schema)?;
    let reader = builder.build()?;

    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let roots = cast_string_col(&batch, FILE_SESSIONS, 0, "root")?;
        let classes = cast_string_col(&batch, FILE_SESSIONS, 1, "trading_class")?;
        let regular_open = cast_i64_col(&batch, FILE_SESSIONS, 2, "regular_open_us")?;
        let regular_close = cast_i64_col(&batch, FILE_SESSIONS, 3, "regular_close_us")?;
        let pre_open = cast_i64_col(&batch, FILE_SESSIONS, 4, "pre_open_us")?;
        let pre_close = cast_i64_col(&batch, FILE_SESSIONS, 5, "pre_close_us")?;
        let post_open = cast_i64_col(&batch, FILE_SESSIONS, 6, "post_open_us")?;
        let post_close = cast_i64_col(&batch, FILE_SESSIONS, 7, "post_close_us")?;
        let curb_open = cast_i64_col(&batch, FILE_SESSIONS, 8, "curb_open_us")?;
        let curb_close = cast_i64_col(&batch, FILE_SESSIONS, 9, "curb_close_us")?;
        let gth_open = cast_i64_col(&batch, FILE_SESSIONS, 10, "gth_open_us")?;
        let gth_close = cast_i64_col(&batch, FILE_SESSIONS, 11, "gth_close_us")?;
        let gth_overnight = cast_bool_col(&batch, FILE_SESSIONS, 12, "gth_overnight")?;
        let last_trading_day_close =
            cast_i64_col(&batch, FILE_SESSIONS, 13, "last_trading_day_close_us")?;
        let settlement_am_open = cast_i64_col(&batch, FILE_SESSIONS, 14, "settlement_am_open_us")?;

        for i in 0..batch.num_rows() {
            if roots.is_null(i)
                || classes.is_null(i)
                || regular_open.is_null(i)
                || regular_close.is_null(i)
            {
                return Err(Error::Parquet(format!(
                    "null cell in required column at row {i} in {FILE_SESSIONS}"
                )));
            }
            let row_id = roots.value(i).to_string();
            let pre = optional_window_strict(pre_open, pre_close, i, &row_id, "pre_market")?;
            let post = optional_window_strict(post_open, post_close, i, &row_id, "post_market")?;
            let curb = optional_window_strict(curb_open, curb_close, i, &row_id, "curb")?;
            let gth = optional_window_strict(gth_open, gth_close, i, &row_id, "gth")?;
            let overnight = if gth.is_some() {
                if gth_overnight.is_null(i) {
                    return Err(Error::DataIntegrity {
                        file: FILE_SESSIONS.to_string(),
                        row: row_id,
                        field: "gth_overnight".to_string(),
                        reason: "gth_open/close populated but gth_overnight is null".to_string(),
                    });
                }
                gth_overnight.value(i)
            } else {
                false
            };
            let last_trading_day_close_us = if last_trading_day_close.is_null(i) {
                None
            } else {
                Some(last_trading_day_close.value(i))
            };
            let settlement = if settlement_am_open.is_null(i) {
                Settlement::Pm
            } else {
                Settlement::AmOpen {
                    open_us_of_day: settlement_am_open.value(i),
                }
            };

            rows.push(SessionInfo {
                root: roots.value(i).to_string(),
                trading_class: TradingClass::from_wire(classes.value(i)),
                regular: TimeWindow::new(regular_open.value(i), regular_close.value(i)),
                pre_market: pre,
                post_market: post,
                curb,
                gth,
                gth_overnight: overnight,
                last_trading_day_close_us,
                settlement,
            });
        }
    }
    rows.sort_by(|a, b| {
        a.root
            .cmp(&b.root)
            .then_with(|| a.trading_class.as_wire().cmp(&b.trading_class.as_wire()))
    });
    Ok(rows)
}

/// Decode a paired (open, close) window. Both endpoints must be set or
/// both null; a half-null pair raises [`Error::DataIntegrity`].
fn optional_window_strict(
    open: &Int64Array,
    close: &Int64Array,
    i: usize,
    row_id: &str,
    field: &str,
) -> Result<Option<TimeWindow>> {
    let open_null = open.is_null(i);
    let close_null = close.is_null(i);
    match (open_null, close_null) {
        (true, true) => Ok(None),
        (false, false) => Ok(Some(TimeWindow::new(open.value(i), close.value(i)))),
        (true, false) => Err(Error::DataIntegrity {
            file: FILE_SESSIONS.to_string(),
            row: row_id.to_string(),
            field: field.to_string(),
            reason: "close set but open is null; pair must be both-set or both-null".to_string(),
        }),
        (false, true) => Err(Error::DataIntegrity {
            file: FILE_SESSIONS.to_string(),
            row: row_id.to_string(),
            field: field.to_string(),
            reason: "open set but close is null; pair must be both-set or both-null".to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionInfo, Settlement, TimeWindow, TradingClass};
    use tempfile::tempdir;

    fn fixture_rows() -> Vec<SessionInfo> {
        vec![
            SessionInfo {
                root: "SPX".into(),
                trading_class: TradingClass::OptionsCboeC1,
                regular: TimeWindow::from_clock_et(9, 30, 16, 15),
                pre_market: None,
                post_market: None,
                curb: Some(TimeWindow::from_clock_et(16, 15, 17, 0)),
                gth: Some(TimeWindow::new(
                    20_i64 * 3_600 * 1_000_000 + 15 * 60 * 1_000_000,
                    9_i64 * 3_600 * 1_000_000 + 25 * 60 * 1_000_000,
                )),
                gth_overnight: true,
                last_trading_day_close_us: None,
                settlement: Settlement::AmOpen {
                    open_us_of_day: AM_SET_US,
                },
            },
            SessionInfo {
                root: "SPXW".into(),
                trading_class: TradingClass::OptionsCboeC1,
                regular: TimeWindow::from_clock_et(9, 30, 16, 15),
                pre_market: None,
                post_market: None,
                curb: Some(TimeWindow::from_clock_et(16, 15, 17, 0)),
                gth: Some(TimeWindow::new(
                    20_i64 * 3_600 * 1_000_000 + 15 * 60 * 1_000_000,
                    9_i64 * 3_600 * 1_000_000 + 25 * 60 * 1_000_000,
                )),
                gth_overnight: true,
                last_trading_day_close_us: Some(57_600_000_000),
                settlement: Settlement::Pm,
            },
            SessionInfo {
                root: "AAPL".into(),
                trading_class: TradingClass::EquityNasdaq,
                regular: TimeWindow::from_clock_et(9, 30, 16, 0),
                pre_market: Some(TimeWindow::from_clock_et(4, 0, 9, 30)),
                post_market: Some(TimeWindow::from_clock_et(16, 0, 20, 0)),
                curb: None,
                gth: Some(TimeWindow::from_clock_et(21, 0, 4, 0)),
                gth_overnight: true,
                last_trading_day_close_us: None,
                settlement: Settlement::Pm,
            },
        ]
    }

    /// 09:30 ET in microseconds-of-day — the AM SET print time for
    /// SPX standard third-Friday expirations. Pulled to module scope so
    /// `clippy::items_after_statements` stays happy in the round-trip
    /// test below.
    const AM_SET_US: i64 = 9 * 3_600 * 1_000_000 + 30 * 60 * 1_000_000;

    #[test]
    fn round_trip_preserves_full_session_payload() -> Result<()> {
        let dir = tempdir()?;
        let rows = fixture_rows();
        write_sessions(dir.path(), &rows)?;
        let back = read_sessions(&dir.path().join(FILE_SESSIONS))?;
        assert_eq!(back.len(), 3);
        assert!(back
            .iter()
            .any(|r| r.root == "AAPL" && r.pre_market.is_some()));
        assert!(back
            .iter()
            .any(|r| r.root == "SPX" && r.curb.is_some() && r.gth_overnight));
        // SPXW carries the last-trading-day close override; SPX does not.
        let spx = back
            .iter()
            .find(|r| r.root == "SPX")
            .expect("SPX in fixture");
        assert_eq!(spx.last_trading_day_close_us, None);
        // SPX carries the AM SET settlement convention; AmOpen at 09:30 ET.
        assert_eq!(
            spx.settlement,
            Settlement::AmOpen {
                open_us_of_day: AM_SET_US,
            },
        );
        let spxw = back
            .iter()
            .find(|r| r.root == "SPXW")
            .expect("SPXW in fixture");
        assert_eq!(spxw.last_trading_day_close_us, Some(57_600_000_000));
        // SPXW is PM-settled regardless of expiration weekday.
        assert_eq!(spxw.settlement, Settlement::Pm);
        let aapl = back
            .iter()
            .find(|r| r.root == "AAPL")
            .expect("AAPL in fixture");
        assert_eq!(aapl.settlement, Settlement::Pm);
        Ok(())
    }

    #[test]
    fn schema_mismatch_is_typed_error() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join(FILE_SESSIONS);
        let bogus_schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let bogus_batch = RecordBatch::try_new(
            Arc::clone(&bogus_schema),
            vec![Arc::new(arrow::array::Int32Array::from(vec![1, 2, 3]))],
        )?;
        let file = fs::File::create(&path)?;
        let mut writer = ArrowWriter::try_new(file, bogus_schema, Some(writer_props()?))?;
        writer.write(&bogus_batch)?;
        writer.close()?;

        let result = read_sessions(&path);
        match result {
            Err(Error::SchemaMismatch { file, .. }) => {
                assert_eq!(file, FILE_SESSIONS);
            }
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn nullability_mismatch_is_schema_mismatch() -> Result<()> {
        // Build a schema identical to the canonical sessions schema
        // EXCEPT every nullable field is declared non-null. The
        // validator must reject it as SchemaMismatch.
        let dir = tempdir()?;
        let path = dir.path().join(FILE_SESSIONS);
        let wrong_schema = Arc::new(Schema::new(vec![
            Field::new("root", DataType::Utf8, false),
            Field::new("trading_class", DataType::Utf8, false),
            Field::new("regular_open_us", DataType::Int64, false),
            Field::new("regular_close_us", DataType::Int64, false),
            Field::new("pre_open_us", DataType::Int64, false), // SHOULD be nullable
            Field::new("pre_close_us", DataType::Int64, false), // SHOULD be nullable
            Field::new("post_open_us", DataType::Int64, false),
            Field::new("post_close_us", DataType::Int64, false),
            Field::new("curb_open_us", DataType::Int64, false),
            Field::new("curb_close_us", DataType::Int64, false),
            Field::new("gth_open_us", DataType::Int64, false),
            Field::new("gth_close_us", DataType::Int64, false),
            Field::new("gth_overnight", DataType::Boolean, false),
            Field::new("last_trading_day_close_us", DataType::Int64, false),
            Field::new("settlement_am_open_us", DataType::Int64, false),
        ]));
        let zeros: Int64Array = std::iter::repeat(Some(0_i64)).take(1).collect();
        let bools: BooleanArray = std::iter::repeat(Some(false)).take(1).collect();
        let strs: StringArray = std::iter::repeat(Some("X")).take(1).collect();
        let columns: Vec<Arc<dyn Array>> = vec![
            Arc::new(strs.clone()),
            Arc::new(strs),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(zeros.clone()),
            Arc::new(bools),
            Arc::new(zeros.clone()),
            Arc::new(zeros),
        ];
        let batch = RecordBatch::try_new(Arc::clone(&wrong_schema), columns)?;
        let file = fs::File::create(&path)?;
        let mut writer = ArrowWriter::try_new(file, wrong_schema, Some(writer_props()?))?;
        writer.write(&batch)?;
        writer.close()?;

        let result = read_sessions(&path);
        assert!(
            matches!(result, Err(Error::SchemaMismatch { .. })),
            "expected SchemaMismatch on nullability mismatch, got {result:?}"
        );
        Ok(())
    }

    #[test]
    fn half_null_window_pair_is_data_integrity_error() -> Result<()> {
        // Pre_open populated, pre_close null on the same row — the
        // reader must surface DataIntegrity rather than silently
        // collapse to None.
        let dir = tempdir()?;
        let path = dir.path().join(FILE_SESSIONS);
        let schema = sessions_schema();
        let roots: StringArray = std::iter::once(Some("X")).collect();
        let classes: StringArray = std::iter::once(Some("equity.nasdaq")).collect();
        let regular_open: Int64Array = std::iter::once(Some(0_i64)).collect();
        let regular_close: Int64Array = std::iter::once(Some(0_i64)).collect();
        let pre_open: Int64Array = std::iter::once(Some(123_i64)).collect();
        let pre_close: Int64Array = std::iter::once(Option::<i64>::None).collect();
        let null_int: Int64Array = std::iter::once(Option::<i64>::None).collect();
        let null_bool: BooleanArray = std::iter::once(Option::<bool>::None).collect();
        let columns: Vec<Arc<dyn Array>> = vec![
            Arc::new(roots),
            Arc::new(classes),
            Arc::new(regular_open),
            Arc::new(regular_close),
            Arc::new(pre_open),
            Arc::new(pre_close),
            Arc::new(null_int.clone()),
            Arc::new(null_int.clone()),
            Arc::new(null_int.clone()),
            Arc::new(null_int.clone()),
            Arc::new(null_int.clone()),
            Arc::new(null_int.clone()),
            Arc::new(null_bool),
            Arc::new(null_int.clone()),
            Arc::new(null_int),
        ];
        let batch = RecordBatch::try_new(Arc::clone(&schema), columns)?;
        let file = fs::File::create(&path)?;
        let mut writer = ArrowWriter::try_new(file, schema, Some(writer_props()?))?;
        writer.write(&batch)?;
        writer.close()?;

        let result = read_sessions(&path);
        match result {
            Err(Error::DataIntegrity { field, .. }) => assert_eq!(field, "pre_market"),
            other => panic!("expected DataIntegrity for pre_market, got {other:?}"),
        }
        Ok(())
    }
}
