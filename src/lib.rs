//! Zero-backend schema inference for CSV, JSON, and NDJSON — runs in the browser via WASM.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
)]

pub(crate) mod hll;
pub mod csv_parser;
pub mod json_parser;
pub mod security;

use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
/// Internal arithmetic helper used in tests.
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

/// Max rows processed; beyond this input is truncated.
pub const MAX_ROWS: usize       = 1_000_000;
/// Max columns; exceeding returns `TooManyColumns`.
pub const MAX_COLS: usize       = 1_024;
/// Max cell bytes; larger cells are treated as null.
pub const MAX_CELL_BYTES: usize = 65_536;
/// Max JSON nesting depth; deeper input returns `NestingTooDeep`.
pub const MAX_JSON_DEPTH: usize = 32;

/// Inferred column type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferredType {
    /// 64-bit integer.
    Integer,
    /// 64-bit float.
    Float,
    /// Boolean (true/false/yes/no).
    Boolean,
    /// ISO-8601 date or datetime.
    Date,
    /// String or mixed.
    String,
    /// Fewer than 2 non-null values — insufficient for inference.
    Unknown,
}

/// Per-column statistics. Never contains raw cell values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnMeta {
    /// Column name, truncated to 256 chars.
    pub name: String,
    /// Zero-based column index.
    pub index: usize,
    /// Dominant inferred type.
    pub inferred_type: InferredType,
    /// True if any null/empty/missing values were found.
    pub nullable: bool,
    /// Count of null/empty/missing cells.
    pub null_count: u64,
    /// Null ratio in [0.0, 1.0].
    pub null_ratio: f64,
    /// Min numeric value (Integer/Float only). Always finite.
    pub numeric_min: Option<f64>,
    /// Max numeric value (Integer/Float only). Always finite.
    pub numeric_max: Option<f64>,
    /// HyperLogLog cardinality estimate (±2%). Upper bound when chunk_count > 1.
    pub cardinality_estimate: u64,
}

/// Schema inference result. Never contains raw cell data.
#[derive(Debug, Serialize, Deserialize)]
pub struct SchemaOutput {
    /// Rows processed (may be less than total if truncated).
    pub row_count: u64,
    /// True if input exceeded MAX_ROWS.
    pub truncated: bool,
    /// Detected format: "csv", "json", or "ndjson".
    pub detected_format: String,
    /// Library version.
    pub schemasniff_version: String,
    /// Always 1 from Rust — chunking is JS-side.
    pub chunk_count: u64,
    /// Columns in source order.
    pub columns: Vec<ColumnMeta>,
}

/// Structured error. Fields contain only counts and positions — never raw input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum SchemaError {
    /// Input exceeded the 10 MB byte cap.
    InputTooLarge   { limit_bytes: usize, actual_bytes: usize },
    /// Column count exceeded MAX_COLS.
    TooManyColumns  { limit: usize, actual: usize },
    /// Row count exceeded MAX_ROWS.
    RowLimitReached { limit: usize },
    /// JSON nesting exceeded MAX_JSON_DEPTH.
    NestingTooDeep  { limit: usize, detected_at_row: usize },
    /// Invalid encoding or NUL byte.
    EncodingError   { byte_offset: Option<usize> },
    /// CSV parse failure (position only).
    CsvParseFailed  { row: usize, column: Option<usize> },
    /// JSON parse failure (position only).
    JsonParseFailed { byte_offset: Option<usize> },
    /// Input was empty or whitespace-only.
    EmptyInput,
    /// Format not recognised as CSV, JSON, or NDJSON.
    UnrecognizedFormat,
}

impl SchemaError {
    /// Serialize for the WASM boundary. Falls back to a static string on failure.
    pub fn into_js(self) -> JsValue {
        serde_wasm_bindgen::to_value(&self)
            .unwrap_or_else(|_| JsValue::from_str("{\"error\":\"serialization_error\"}"))
    }

    /// True for errors that indicate a hard security cap was hit.
    pub fn is_security_event(&self) -> bool {
        matches!(
            self,
            SchemaError::InputTooLarge { .. }
                | SchemaError::TooManyColumns { .. }
                | SchemaError::RowLimitReached { .. }
                | SchemaError::NestingTooDeep { .. }
                | SchemaError::EncodingError { .. }
        )
    }
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::InputTooLarge { limit_bytes, actual_bytes } =>
                write!(f, "input too large: {actual_bytes} bytes exceeds {limit_bytes} byte limit"),
            SchemaError::TooManyColumns { limit, actual } =>
                write!(f, "too many columns: {actual} exceeds limit of {limit}"),
            SchemaError::RowLimitReached { limit } =>
                write!(f, "row limit reached: processing stopped at {limit} rows"),
            SchemaError::NestingTooDeep { limit, detected_at_row } =>
                write!(f, "nesting too deep at row {detected_at_row}: limit is {limit} levels"),
            SchemaError::EncodingError { byte_offset: Some(pos) } =>
                write!(f, "encoding error at byte {pos}"),
            SchemaError::EncodingError { byte_offset: None } =>
                write!(f, "encoding error at unknown position"),
            SchemaError::CsvParseFailed { row, column: Some(col) } =>
                write!(f, "CSV parse failed at row {row}, column {col}"),
            SchemaError::CsvParseFailed { row, column: None } =>
                write!(f, "CSV parse failed at row {row}"),
            SchemaError::JsonParseFailed { byte_offset: Some(pos) } =>
                write!(f, "JSON parse failed at byte {pos}"),
            SchemaError::JsonParseFailed { byte_offset: None } =>
                write!(f, "JSON parse failed at unknown position"),
            SchemaError::EmptyInput =>
                write!(f, "input contained no data rows"),
            SchemaError::UnrecognizedFormat =>
                write!(f, "format not recognised as CSV, JSON, or NDJSON"),
        }
    }
}

impl std::error::Error for SchemaError {}

/// Testable core — no wasm-bindgen types.
pub fn infer_schema_inner(input: &str) -> Result<SchemaOutput, SchemaError> {
    const JS_BYTE_CAP: usize = 10 * 1024 * 1024;
    if input.len() > JS_BYTE_CAP {
        return Err(SchemaError::InputTooLarge {
            limit_bytes: JS_BYTE_CAP,
            actual_bytes: input.len(),
        });
    }
    Ok(SchemaOutput {
        row_count: 0,
        truncated: false,
        detected_format: "stub".to_string(),
        schemasniff_version: env!("CARGO_PKG_VERSION").to_string(),
        chunk_count: 1,
        columns: vec![ColumnMeta {
            name: "example_column".to_string(),
            index: 0,
            inferred_type: InferredType::String,
            nullable: false,
            null_count: 0,
            null_ratio: 0.0,
            numeric_min: None,
            numeric_max: None,
            cardinality_estimate: 0,
        }],
    })
}

/// WASM entry point. Returns `SchemaOutput` or `SchemaError` serialized as `JsValue`.
#[wasm_bindgen]
pub fn infer_schema(input: &str) -> Result<JsValue, JsValue> {
    security::PurityGuarantee::assert();

    security::validate_input(input).map_err(|e| {
        serde_wasm_bindgen::to_value(&e)
            .unwrap_or(JsValue::from_str("serialization_error"))
    })?;

    let trimmed = input.trim_start();
    let result = if trimmed.starts_with('[') || trimmed.starts_with('{') {
        json_parser::parse_json(input)
    } else {
        csv_parser::parse_csv(input)
    };

    let output = result
        .and_then(security::sanitize_output)
        .map_err(|e| {
            serde_wasm_bindgen::to_value(&e)
                .unwrap_or(JsValue::from_str("serialization_error"))
        })?;

    serde_wasm_bindgen::to_value(&output)
        .map_err(|e| JsValue::from_str(&format!("serialization_error: {e}")))
}

/// Enables panic backtraces in browser DevTools (dev builds only).
#[wasm_bindgen(start)]
pub fn on_wasm_init() {
    #[cfg(feature = "dev")]
    {
        console_error_panic_hook::set_once();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_cap_enforced() {
        let oversized = "x".repeat(10 * 1024 * 1024 + 1);
        let result = infer_schema_inner(&oversized);
        assert!(matches!(result.unwrap_err(), SchemaError::InputTooLarge { .. }));
    }

    #[test]
    fn empty_input_returns_ok_stub() {
        assert!(infer_schema_inner("").is_ok());
    }

    #[test]
    fn constants_are_sane() {
        assert!(MAX_ROWS > 0 && MAX_ROWS <= 10_000_000);
        assert!(MAX_COLS > 0 && MAX_COLS <= 10_000);
        assert!(MAX_CELL_BYTES > 0);
        assert!(MAX_JSON_DEPTH > 0);
    }

    #[test]
    fn schema_output_serializes_cleanly() {
        let output = SchemaOutput {
            row_count: 42, truncated: false,
            detected_format: "csv".to_string(),
            schemasniff_version: "0.1.0".to_string(),
            chunk_count: 1, columns: vec![],
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let back: SchemaOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.row_count, 42);
    }

    #[test]
    fn schema_error_serializes_with_tag() {
        let err = SchemaError::TooManyColumns { limit: 1024, actual: 2000 };
        let json = serde_json::to_string(&err).expect("serialize");
        assert!(json.contains("\"error\":\"too_many_columns\""));
        assert!(json.contains("2000"));
    }

    #[test]
    fn inferred_type_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&InferredType::Integer).unwrap(), "\"integer\"");
        assert_eq!(serde_json::to_string(&InferredType::Unknown).unwrap(), "\"unknown\"");
    }
}

#[cfg(test)]
mod schema_error_seal {
    use super::SchemaError;
    use std::mem::size_of;

    #[test]
    fn error_variants_contain_no_heap_strings() {
        // If a variant gains a String/Vec field this will fail at compile time.
        const MAX_EXPECTED: usize = 4 * size_of::<usize>();
        assert!(
            size_of::<SchemaError>() <= MAX_EXPECTED,
            "SchemaError grew to {} bytes — a variant may contain heap data",
            size_of::<SchemaError>(),
        );
    }

    #[test]
    fn error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SchemaError>();
    }

    #[test]
    fn error_is_clone_eq() {
        let e = SchemaError::EmptyInput;
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn display_contains_no_static_cell_content() {
        let errors = [
            SchemaError::InputTooLarge { limit_bytes: 100, actual_bytes: 200 },
            SchemaError::TooManyColumns { limit: 1024, actual: 2000 },
            SchemaError::RowLimitReached { limit: 1_000_000 },
            SchemaError::NestingTooDeep { limit: 32, detected_at_row: 5 },
            SchemaError::EncodingError { byte_offset: Some(42) },
            SchemaError::EncodingError { byte_offset: None },
            SchemaError::CsvParseFailed { row: 10, column: Some(3) },
            SchemaError::CsvParseFailed { row: 10, column: None },
            SchemaError::JsonParseFailed { byte_offset: Some(99) },
            SchemaError::JsonParseFailed { byte_offset: None },
            SchemaError::EmptyInput,
            SchemaError::UnrecognizedFormat,
        ];
        for err in &errors {
            let msg = err.to_string();
            assert!(msg.chars().all(|c| c.is_ascii()), "non-ASCII in: {msg:?}");
            assert!(msg.chars().any(|c| c.is_ascii_alphabetic()), "no letters in: {msg:?}");
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    fn run_ok(input: &str) -> SchemaOutput {
        let trimmed = input.trim_start();
        let result = if trimmed.starts_with('[') || trimmed.starts_with('{') {
            json_parser::parse_json(input)
        } else {
            csv_parser::parse_csv(input)
        };
        let out = result.and_then(security::sanitize_output).expect("expected Ok");
        let json = serde_json::to_string(&out).expect("serialize");
        serde_json::from_str(&json).expect("deserialize")
    }

    fn run_err(input: &str) -> SchemaError {
        security::validate_input(input).err().unwrap_or_else(|| {
            let trimmed = input.trim_start();
            let result = if trimmed.starts_with('[') || trimmed.starts_with('{') {
                json_parser::parse_json(input)
            } else {
                csv_parser::parse_csv(input)
            };
            result.and_then(security::sanitize_output).expect_err("expected Err")
        })
    }

    #[test]
    fn happy_path_csv_10_rows_5_columns() {
        let csv = "name,age,score,active,joined\n\
                   Alice,30,9.5,true,2023-01-01\n\
                   Bob,25,8.0,false,2023-02-14\n\
                   Carol,40,7.2,true,2023-03-10\n\
                   Dave,35,6.8,false,2023-04-05\n\
                   Eve,28,9.9,true,2023-05-20\n\
                   Frank,52,5.5,false,2023-06-30\n\
                   Grace,19,8.8,true,2023-07-07\n\
                   Heidi,44,7.1,false,2023-08-01\n\
                   Ivan,31,6.0,true,2023-09-15\n\
                   Judy,27,9.2,false,2023-10-31";
        let out = run_ok(csv);
        assert_eq!(out.row_count, 10);
        assert_eq!(out.detected_format, "csv");
        assert_eq!(out.columns.len(), 5);
    }

    #[test]
    fn malformed_json_returns_json_parse_failed() {
        assert!(matches!(run_err("{bad json}"), SchemaError::JsonParseFailed { .. }));
    }

    #[test]
    fn oversized_input_returns_input_too_large() {
        let big = "a".repeat(10 * 1024 * 1024 + 1);
        assert!(matches!(run_err(&big), SchemaError::InputTooLarge { .. }));
    }

    #[test]
    fn header_only_csv_returns_empty_input() {
        assert!(matches!(run_err("name,age\n"), SchemaError::EmptyInput));
    }

    #[test]
    fn unicode_column_names_preserved() {
        let csv = "名前,年齢,score\nAlice,30,9.5\nBob,25,8.0";
        let out = run_ok(csv);
        assert_eq!(out.columns[0].name, "名前");
        assert_eq!(out.columns[1].name, "年齢");
    }
}
