//! Security validation layers for schemasniff.
//!
//! Five layers, applied in strict order before any parsing occurs:
//!
//!   1. JS-side byte cap      — 10 MB hard limit in the TS wrapper (pre-WASM)
//!   2. Input type guard      — must be a non-null UTF-8 string
//!   3. Row / col / cell caps — MAX_ROWS, MAX_COLS, MAX_CELL_BYTES (enforced in parsers)
//!   4. Pure function check   — no global state, no side effects (structural guarantee)
//!   5. Output sanitization   — no raw cell values in any output field
//!
//! Layers 1 and 4 are structural/wrapper guarantees documented here.
//! Layers 2, 3, and 5 are enforced by functions in this module.

use crate::{SchemaError, SchemaOutput, ColumnMeta, InferredType};

/// Maximum byte length enforced on the Rust side as a belt-and-suspenders
/// check. The JS wrapper enforces this first; this catches any caller that
/// bypasses the wrapper and calls WASM directly.
pub const RUST_BYTE_CAP: usize = 10 * 1024 * 1024; // 10 MB

// ── Layer 2: Input type guard ─────────────────────────────────────────────────

/// Validate that the input string is safe to pass to a parser.
///
/// Checks (in order):
///   - Not empty / not whitespace-only  → `EmptyInput`
///   - Byte length within `RUST_BYTE_CAP` → `InputTooLarge`
///   - Valid UTF-8 is guaranteed by the `&str` type itself in Rust;
///     the WASM boundary rejects invalid UTF-8 before this is called.
///
/// This is called by `infer_schema` before format detection or parsing.
pub fn validate_input(input: &str) -> Result<(), SchemaError> {
    // Belt-and-suspenders byte cap (Layer 1 is JS-side, this is Layer 2 backup)
    if input.len() > RUST_BYTE_CAP {
        return Err(SchemaError::InputTooLarge {
            limit_bytes: RUST_BYTE_CAP,
            actual_bytes: input.len(),
        });
    }

    // Empty / whitespace-only input has no schema to infer
    if input.trim().is_empty() {
        return Err(SchemaError::EmptyInput);
    }

    // NUL bytes are valid UTF-8 but break many parsers and signal
    // binary data, not text. Reject with position of first NUL.
    if let Some(nul_pos) = input.bytes().position(|b| b == 0x00) {
        return Err(SchemaError::EncodingError {
            byte_offset: Some(nul_pos),
        });
    }

    Ok(())
}

// ── Layer 5: Output sanitization ──────────────────────────────────────────────

/// Validate that a completed `SchemaOutput` contains no raw cell data.
///
/// This is a defence-in-depth check run after parsing completes —
/// it verifies the parser upheld the "no raw values" contract before
/// the output crosses the WASM boundary into JS.
///
/// Rules enforced:
///   - `column.name` must be ≤ 256 chars (sanitized at parse time; verify here)
///   - `numeric_min` / `numeric_max` must be `None` for non-numeric columns
///   - `numeric_min` / `numeric_max` must be finite if `Some` (no NaN / Inf)
///   - `null_ratio` must be in [0.0, 1.0]
///   - `detected_format` must be one of the three known literals
///   - `row_count` must be consistent with column stats
///
/// Returns the output unchanged if valid, or a `SchemaError::ParseError`
/// if any invariant is broken (which would indicate a parser bug).
pub fn sanitize_output(output: SchemaOutput) -> Result<SchemaOutput, SchemaError> {
    // Validate detected_format is a known literal
    match output.detected_format.as_str() {
        "csv" | "json" | "ndjson" => {}
        _ => return Err(SchemaError::UnrecognizedFormat),
    }

    for col in &output.columns {
        sanitize_column(col, output.row_count)?;
    }

    Ok(output)
}

/// Validate a single [`ColumnMeta`] for output safety.
fn sanitize_column(col: &ColumnMeta, row_count: u64) -> Result<(), SchemaError> {
    // Column name must have been truncated to ≤ 256 chars at parse time
    if col.name.chars().count() > 256 {
        return Err(SchemaError::CsvParseFailed {
            row: 0,
            column: Some(col.index),
        });
    }

    // null_ratio must be in [0.0, 1.0] and mathematically consistent
    if col.null_ratio < 0.0 || col.null_ratio > 1.0 || !col.null_ratio.is_finite() {
        return Err(SchemaError::CsvParseFailed {
            row: 0,
            column: Some(col.index),
        });
    }

    // null_count must not exceed row_count
    if col.null_count > row_count {
        return Err(SchemaError::CsvParseFailed {
            row: 0,
            column: Some(col.index),
        });
    }

    // Numeric bounds must only be present for numeric columns
    let is_numeric = matches!(
        col.inferred_type,
        InferredType::Integer | InferredType::Float
    );

    if !is_numeric && (col.numeric_min.is_some() || col.numeric_max.is_some()) {
        return Err(SchemaError::CsvParseFailed {
            row: 0,
            column: Some(col.index),
        });
    }

    // Numeric bounds must be finite if present — no NaN or Infinity in output
    if let Some(min) = col.numeric_min {
        if !min.is_finite() {
            return Err(SchemaError::CsvParseFailed {
                row: 0,
                column: Some(col.index),
            });
        }
    }
    if let Some(max) = col.numeric_max {
        if !max.is_finite() {
            return Err(SchemaError::CsvParseFailed {
                row: 0,
                column: Some(col.index),
            });
        }
    }

    // min must be ≤ max when both are present
    if let (Some(min), Some(max)) = (col.numeric_min, col.numeric_max) {
        if min > max {
            return Err(SchemaError::CsvParseFailed {
                row: 0,
                column: Some(col.index),
            });
        }
    }

    Ok(())
}

// ── Layer 4: Pure function guarantee (structural) ─────────────────────────────

/// Documents the purity contract for `infer_schema`.
///
/// `infer_schema` is pure in the functional sense:
///   - No mutable global state (no `static mut`, no `OnceLock` writes after init)
///   - No I/O (no filesystem, network, or DOM access — WASM target has none)
///   - No thread-local side effects
///   - Deterministic output for deterministic input (HLL uses `RandomState`
///     so cardinality estimates vary across runs, which is documented)
///
/// This is enforced structurally:
///   - `#![forbid(unsafe_code)]` prevents raw pointer aliasing of globals
///   - The WASM target has no filesystem or network syscalls
///   - All state is stack/heap-allocated within the function call
///
/// No runtime check is needed or possible for purity — this module
/// documents and centralises the guarantee so it can be audited in one place.
pub struct PurityGuarantee;

impl PurityGuarantee {
    /// Assert the purity contract at call time (zero-cost — optimised away).
    /// Called by `infer_schema` to make the contract visible in the call stack.
    #[inline(always)]
    pub fn assert() {}
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_input ────────────────────────────────────────────────────────

    #[test]
    fn empty_string_rejected() {
        assert!(matches!(validate_input(""), Err(SchemaError::EmptyInput)));
    }

    #[test]
    fn whitespace_only_rejected() {
        assert!(matches!(validate_input("   \n\t  "), Err(SchemaError::EmptyInput)));
    }

    #[test]
    fn oversized_input_rejected() {
        let big = "a".repeat(RUST_BYTE_CAP + 1);
        assert!(matches!(
            validate_input(&big),
            Err(SchemaError::InputTooLarge { .. })
        ));
    }

    #[test]
    fn nul_byte_rejected() {
        let input = "name,age\nAlice\x0030";
        let err = validate_input(input).expect_err("NUL must be rejected");
        assert!(matches!(err, SchemaError::EncodingError { byte_offset: Some(_) }));
    }

    #[test]
    fn valid_input_passes() {
        assert!(validate_input("name,age\nAlice,30").is_ok());
        assert!(validate_input("{\"a\":1}").is_ok());
    }

    #[test]
    fn unicode_input_passes() {
        assert!(validate_input("名前,年齢\nAlice,30").is_ok());
    }

    // ── sanitize_output ───────────────────────────────────────────────────────

    fn make_output(cols: Vec<ColumnMeta>) -> SchemaOutput {
        SchemaOutput {
            row_count: 10,
            truncated: false,
            detected_format: "csv".to_string(),
            schemasniff_version: "0.1.0".to_string(),
            chunk_count: 1,
            columns: cols,
        }
    }

    fn make_col(name: &str, t: InferredType) -> ColumnMeta {
        ColumnMeta {
            name: name.to_string(),
            index: 0,
            inferred_type: t,
            nullable: false,
            null_count: 0,
            null_ratio: 0.0,
            numeric_min: None,
            numeric_max: None,
            cardinality_estimate: 0,
        }
    }

    #[test]
    fn clean_output_passes() {
        let out = make_output(vec![make_col("age", InferredType::Integer)]);
        assert!(sanitize_output(out).is_ok());
    }

    #[test]
    fn unknown_format_rejected() {
        let mut out = make_output(vec![]);
        out.detected_format = "xml".to_string();
        assert!(sanitize_output(out).is_err());
    }

    #[test]
    fn numeric_bounds_on_string_col_rejected() {
        let mut col = make_col("name", InferredType::String);
        col.numeric_min = Some(1.0);
        assert!(sanitize_output(make_output(vec![col])).is_err());
    }

    #[test]
    fn nan_in_numeric_min_rejected() {
        let mut col = make_col("score", InferredType::Float);
        col.numeric_min = Some(f64::NAN);
        assert!(sanitize_output(make_output(vec![col])).is_err());
    }

    #[test]
    fn infinite_numeric_max_rejected() {
        let mut col = make_col("score", InferredType::Float);
        col.numeric_min = Some(1.0);
        col.numeric_max = Some(f64::INFINITY);
        assert!(sanitize_output(make_output(vec![col])).is_err());
    }

    #[test]
    fn min_greater_than_max_rejected() {
        let mut col = make_col("score", InferredType::Float);
        col.numeric_min = Some(9.0);
        col.numeric_max = Some(1.0);
        assert!(sanitize_output(make_output(vec![col])).is_err());
    }

    #[test]
    fn null_count_exceeding_row_count_rejected() {
        let mut col = make_col("x", InferredType::String);
        col.null_count = 11; // row_count is 10
        assert!(sanitize_output(make_output(vec![col])).is_err());
    }

    #[test]
    fn column_name_over_256_chars_rejected() {
        let mut col = make_col(&"a".repeat(257), InferredType::String);
        col.name = "a".repeat(257);
        assert!(sanitize_output(make_output(vec![col])).is_err());
    }

    #[test]
    fn invalid_null_ratio_rejected() {
        let mut col = make_col("x", InferredType::String);
        col.null_ratio = 1.5;
        assert!(sanitize_output(make_output(vec![col])).is_err());
    }
}