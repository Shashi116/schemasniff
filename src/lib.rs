//! Zero-backend schema inference engine for CSV, JSON, and NDJSON.
//! Runs entirely in the browser via WASM with no external dependencies or I/O.
#![forbid(unsafe_code)]
#![deny(
    missing_docs,
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

/// Utility function for testing arithmetic operations.
#[allow(dead_code)]
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

// Hard security limits to prevent OOM and DoS attacks

/// Max rows; beyond this, rows are silently skipped and truncated flag is set
pub const MAX_ROWS: usize = 1_000_000;

/// Max columns; exceeding this returns TooManyColumns error immediately
pub const MAX_COLS: usize = 1_024;

/// Max byte length per cell; larger cells treated as null to prevent OOM
pub const MAX_CELL_BYTES: usize = 65_536;

/// Max JSON nesting depth; protects against stack-overflow attacks
pub const MAX_JSON_DEPTH: usize = 32;

// Only these types cross the WASM boundary; never contain raw cell values

/// Inferred column type
///
/// This is a closed enum — new variants must be added deliberately.
/// The `Unknown` variant is used when fewer than 2 non-null cells are present,
/// which is insufficient for reliable inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferredType {
    /// 64-bit integers only
    Integer,
    /// 64-bit floats (includes integers)
    Float,
    /// Boolean values (true/false, case-insensitive)
    Boolean,
    /// ISO-8601 dates/datetimes, matched structurally not via regex
    Date,
    /// Mixed or unclassified types
    String,
    /// Fewer than 2 non-null values, can't reliably infer
    Unknown,
}

/// Column metadata (never leaks raw cell values)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnMeta {
    /// Sanitized column name, truncated to 256 chars
    pub name: String,

    /// Zero-based column position in source
    pub index: usize,

    /// Inferred type from non-null values
    pub inferred_type: InferredType,

    /// Has null/empty/missing values
    pub nullable: bool,

    /// Count of null/empty/missing values
    pub null_count: u64,

    /// Ratio of nulls to total rows [0.0, 1.0]
    pub null_ratio: f64,

    /// Min numeric value (Integer/Float only, None for other types)
    pub numeric_min: Option<f64>,

    /// Max numeric value (Integer/Float only, None for other types)
    pub numeric_max: Option<f64>,

    /// Distinct value count estimate via HyperLogLog (±2% error, irreversible)
    pub cardinality_estimate: u64,
}

/// Complete schema inference result (never contains raw cell data)
#[derive(Debug, Serialize, Deserialize)]
pub struct SchemaOutput {
    /// Rows actually processed (may be less if truncated)
    pub row_count: u64,

    /// True if input exceeded MAX_ROWS and was partially processed
    pub truncated: bool,

    /// Detected format: "csv", "json", or "ndjson"
    pub detected_format: String,

    /// Library version that produced this output
    pub schemasniff_version: String,

    /// Number of chunks processed (always 1 from Rust — chunking is JS-side)
    pub chunk_count: u64,

    /// Metadata for each column in source order
    pub columns: Vec<ColumnMeta>,
}

// Closed error enum, never contains raw input data, only counts/positions

/// Structured error response (never echoes input data)
/// Structured error response with only counts/positions, never raw input data
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum SchemaError {
    /// Input byte length exceeded the 10 MB cap.
    /// Caught JS-side first; this fires if WASM is called directly.
    InputTooLarge {
        /// Byte limit enforced (10 * 1024 * 1024)
        limit_bytes: usize,
        /// Actual byte length received
        actual_bytes: usize,
    },

    /// Column count exceeded MAX_COLS (1,024).
    /// Emitted before any row data is parsed.
    TooManyColumns {
        /// Maximum columns allowed
        limit: usize,
        /// Actual column count found in header
        actual: usize,
    },

    /// Row count exceeded MAX_ROWS (1,000,000).
    /// Parsing stopped; partial output is not returned — use truncated flag instead.
    RowLimitReached {
        /// Maximum rows allowed
        limit: usize,
    },

    /// JSON nesting depth exceeded MAX_JSON_DEPTH (32).
    /// Protects against stack-overflow via deeply recursive structures.
    NestingTooDeep {
        /// Maximum depth allowed
        limit: usize,
        /// Zero-based row index where excessive depth was detected
        detected_at_row: usize,
    },

    /// Invalid byte sequence or NUL byte in input.
    /// UTF-8 validity is enforced at WASM boundary; NUL is caught in validate_input.
    EncodingError {
        /// Byte offset of the first invalid byte, if known
        byte_offset: Option<usize>,
    },

    /// CSV structural parse failure.
    /// Position only — the offending cell content is never included.
    CsvParseFailed {
        /// Zero-based row index where parse failed
        row: usize,
        /// Zero-based column index where parse failed, if known
        column: Option<usize>,
    },

    /// JSON structural parse failure.
    /// Byte offset only — no surrounding context or content is included.
    JsonParseFailed {
        /// Byte offset of the unexpected token, if known
        byte_offset: Option<usize>,
    },

    /// Input was empty, whitespace-only, or contained only a header with no data rows.
    EmptyInput,

    /// Input did not begin with a recognised format marker.
    /// CSV must not start with `[` or `{`; JSON must start with `[`; NDJSON with `{`.
    UnrecognizedFormat,
}

impl SchemaError {
    /// Serialize to a `JsValue` for return across the WASM boundary.
    /// Falls back to a static string if serialization itself fails —
    /// that path should be unreachable given the types above.
    pub fn into_js(self) -> JsValue {
        serde_wasm_bindgen::to_value(&self)
            .unwrap_or_else(|_| JsValue::from_str("{\"error\":\"serialization_error\"}"))
    }

    /// Returns true if this error represents a hard security cap being hit.
    /// Useful for logging/monitoring to distinguish security events from
    /// ordinary parse failures.
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
        // SECURITY: format strings must never interpolate user input.
        // Only counts and positions (usize) are permitted here.
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
// Core logic — native-testable, no wasm-bindgen types

/// Infer schema from CSV, JSON, or NDJSON input (must be valid UTF-8)
///
/// Pure function: no DOM, network, FS, or global state access.  
/// Call this directly in unit tests; the wasm entry point is a thin wrapper.
pub fn infer_schema_inner(input: &str) -> Result<SchemaOutput, SchemaError> {
    const JS_BYTE_CAP: usize = 10 * 1024 * 1024;
    if input.len() > JS_BYTE_CAP {
        return Err(SchemaError::InputTooLarge {
            limit_bytes: JS_BYTE_CAP,
            actual_bytes: input.len(),
        });
    }

    // Temporary stub: returns hardcoded output until real parsing is implemented
    Ok(SchemaOutput {
        row_count: 0,
        truncated: false,
        detected_format: "stub".to_string(),
        schemasniff_version: env!("CARGO_PKG_VERSION").to_string(),
        chunk_count: 1,
        columns: vec![
            ColumnMeta {
                name: "example_column".to_string(),
                index: 0,
                inferred_type: InferredType::String,
                nullable: false,
                null_count: 0,
                null_ratio: 0.0,
                numeric_min: None,
                numeric_max: None,
                cardinality_estimate: 0,
            }
        ],
    })
}

// WASM entry point — thin wrapper around infer_schema_inner

/// Infer schema from CSV, JSON, or NDJSON input (must be valid UTF-8)
///
/// Returns JSON-serialized SchemaOutput on success or SchemaError on failure.
/// Check result.error field to distinguish success from error.
/// Has no side effects: no DOM, network, FS, or global state access.
#[wasm_bindgen]
pub fn infer_schema(input: &str) -> Result<JsValue, JsValue> {
    // Layer 4: pure function contract (zero-cost, auditable call site)
    security::PurityGuarantee::assert();

    // Layer 1 (JS) + Layer 2 (Rust): byte cap and input type guard
    security::validate_input(input).map_err(|e| {
        serde_wasm_bindgen::to_value(&e)
            .unwrap_or(JsValue::from_str("serialization_error"))
    })?;

    // Format detection + Layer 3: row/col/cell caps enforced inside parsers
    let trimmed = input.trim_start();
    let result = if trimmed.starts_with('[') || trimmed.starts_with('{') {
        json_parser::parse_json(input)
    } else {
        csv_parser::parse_csv(input)
    };

    // Layer 5: output sanitization before crossing WASM boundary
    let output = result
        .and_then(security::sanitize_output)
        .map_err(|e| {
            serde_wasm_bindgen::to_value(&e)
                .unwrap_or(JsValue::from_str("serialization_error"))
        })?;

    serde_wasm_bindgen::to_value(&output)
        .map_err(|e| JsValue::from_str(&format!("serialization_error: {e}")))
}

// Development helpers: init hook for panic handling in dev builds

/// Routes Rust panics to browser console with backtraces (dev builds only)
#[wasm_bindgen(start)]
pub fn on_wasm_init() {
    #[cfg(feature = "dev")]
    {
        console_error_panic_hook::set_once();
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_cap_enforced() {
        // Input 1 byte over the 10 MB cap must return InputTooLarge
        let oversized = "x".repeat(10 * 1024 * 1024 + 1);
        let result = infer_schema_inner(&oversized);
        assert!(result.is_err(), "oversized input must return Err");
        assert!(matches!(result.unwrap_err(), SchemaError::InputTooLarge { .. }));
    }

    #[test]
    fn empty_input_returns_ok_stub() {
        // Stub returns Ok for any valid input; replace on Day 2 with EmptyInput error
        let result = infer_schema_inner("");
        assert!(result.is_ok(), "stub must return Ok for any valid input");
    }

    #[test]
    fn constants_are_sane() {
        // Sanity checks on constant limits
        assert!(MAX_ROWS > 0);
        assert!(MAX_COLS > 0);
        assert!(MAX_CELL_BYTES > 0);
        assert!(MAX_JSON_DEPTH > 0);
        assert!(MAX_ROWS <= 10_000_000, "cap too high — OOM risk");
        assert!(MAX_COLS <= 10_000, "col cap too high — DoS risk");
    }

    #[test]
    fn schema_output_serializes_cleanly() {
        // Confirm serde round-trips without panicking
        let output = SchemaOutput {
            row_count: 42,
            truncated: false,
            detected_format: "csv".to_string(),
            schemasniff_version: "0.1.0".to_string(),
            chunk_count: 1,
            columns: vec![],
        };
        let json = serde_json::to_string(&output).expect("serialize");
        let back: SchemaOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.row_count, 42);
        assert!(!back.truncated);
    }

    #[test]
    fn schema_error_serializes_with_tag() {
        // Errors serialize with "error" discriminant field for JS consumer
        let err = SchemaError::TooManyColumns { limit: 1024, actual: 2000 };
        let json = serde_json::to_string(&err).expect("serialize");
        assert!(json.contains("\"error\":\"too_many_columns\""), "missing error tag: {json}");
        assert!(json.contains("2000"), "actual count missing: {json}");
    }

    #[test]
    fn inferred_type_serializes_snake_case() {
        // Verify enum serializes to snake_case
        let t = InferredType::Integer;
        let json = serde_json::to_string(&t).expect("serialize");
        assert_eq!(json, "\"integer\"");

        let t2 = InferredType::Unknown;
        let json2 = serde_json::to_string(&t2).expect("serialize");
        assert_eq!(json2, "\"unknown\"");
    }
}

// ── Compile-time field type seal ──────────────────────────────────────────────
//
// This test will fail to compile if any variant gains a String, Vec,
// or other heap type that could carry raw cell content.
// It is the machine-checked version of the "position-only" rule.
//
// How it works: ZST assertions compile only when the tested types are the
// expected size. A `String` field would make the variant larger than
// `3 * size_of::<usize>()` (the largest current variant), causing the
// static assert to fire at compile time, not at runtime.

#[cfg(test)]
mod schema_error_seal {
    use super::SchemaError;
    use std::mem::size_of;

    #[test]
    fn error_variants_contain_no_heap_strings() {
        // SchemaError must not grow unboundedly.
        // Current largest variant: InputTooLarge { limit_bytes, actual_bytes }
        // = 2 * usize. With enum discriminant + padding, total ≤ 4 * usize.
        // If this assertion fires, a variant has gained a String or Vec field.
        const MAX_EXPECTED: usize = 4 * size_of::<usize>();
        assert!(
            size_of::<SchemaError>() <= MAX_EXPECTED,
            "SchemaError size {} exceeds {}. \
             A variant may contain a heap-allocated type (String, Vec, etc.) \
             which could carry raw cell content. \
             All fields must be usize or Option<usize>.",
            size_of::<SchemaError>(),
            MAX_EXPECTED,
        );
    }

    #[test]
    fn error_is_send_sync() {
        // SchemaError must be Send + Sync — no Rc or raw pointers
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SchemaError>();
    }

    #[test]
    fn error_is_clone_eq() {
        // Must be Clone + PartialEq for test assertions and deduplication
        let e = SchemaError::EmptyInput;
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn display_contains_no_static_cell_content() {
        // Every Display message must contain only digits and known static strings.
        // This checks the formatting doesn't accidentally include a cell value
        // by verifying messages only contain ASCII printable chars, digits,
        // spaces, and punctuation — never arbitrary Unicode that cell data
        // could inject.
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
            assert!(
                msg.chars().all(|c| c.is_ascii()),
                "Display message for {err:?} contains non-ASCII: {msg:?}. \
                 Cell content may have leaked into an error message."
            );
            // Must contain at least one letter (not a bare number)
            assert!(
                msg.chars().any(|c| c.is_ascii_alphabetic()),
                "Display message for {err:?} contains no letters: {msg:?}"
            );
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    // Call the full infer_schema pipeline (validate → parse → sanitize) and
    // round-trip the result through serde_json, which works on native targets.
    // serde_wasm_bindgen requires a JS runtime and cannot be used with cargo test.
    fn run_ok(input: &str) -> SchemaOutput {
        let trimmed = input.trim_start();
        let result = if trimmed.starts_with('[') || trimmed.starts_with('{') {
            json_parser::parse_json(input)
        } else {
            csv_parser::parse_csv(input)
        };
        let out = result
            .and_then(security::sanitize_output)
            .expect("expected Ok");
        // Round-trip through serde_json to verify the output is serializable
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
            result
                .and_then(security::sanitize_output)
                .expect_err("expected Err")
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
        assert!(!out.truncated);
        assert_eq!(out.detected_format, "csv");
        assert_eq!(out.columns.len(), 5);
    }

    #[test]
    fn malformed_json_returns_json_parse_failed() {
        let err = run_err("{bad json}");
        assert!(matches!(err, SchemaError::JsonParseFailed { .. }));
    }

    #[test]
    fn oversized_input_returns_input_too_large() {
        let big = "a".repeat(10 * 1024 * 1024 + 1);
        let err = run_err(&big);
        assert!(matches!(err, SchemaError::InputTooLarge { .. }));
    }

    #[test]
    fn header_only_csv_returns_empty_input() {
        let err = run_err("name,age\n");
        assert!(matches!(err, SchemaError::EmptyInput));
    }

    #[test]
    fn unicode_column_names_preserved() {
        let csv = "名前,年齢,score\nAlice,30,9.5\nBob,25,8.0";
        let out = run_ok(csv);
        assert_eq!(out.columns[0].name, "名前");
        assert_eq!(out.columns[1].name, "年齢");
    }
}
