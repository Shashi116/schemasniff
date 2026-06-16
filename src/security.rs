//! Input validation and output sanitization.
//!
//! Layers applied before any parsing:
//!   1. JS-side byte cap (TS wrapper)
//!   2. Rust-side byte cap + empty/NUL check  ← validate_input
//!   3. Row / col / cell caps                 ← enforced in parsers
//!   4. Purity guarantee                      ← PurityGuarantee::assert
//!   5. Output sanitization                   ← sanitize_output

use crate::{SchemaError, SchemaOutput, ColumnMeta, InferredType};

/// 10 MB hard cap enforced on the Rust side.
pub const RUST_BYTE_CAP: usize = 10 * 1024 * 1024;

/// Reject oversized, empty, or NUL-containing input before parsing.
pub fn validate_input(input: &str) -> Result<(), SchemaError> {
    if input.len() > RUST_BYTE_CAP {
        return Err(SchemaError::InputTooLarge {
            limit_bytes: RUST_BYTE_CAP,
            actual_bytes: input.len(),
        });
    }
    if input.trim().is_empty() {
        return Err(SchemaError::EmptyInput);
    }
    if let Some(pos) = input.bytes().position(|b| b == 0x00) {
        return Err(SchemaError::EncodingError { byte_offset: Some(pos) });
    }
    Ok(())
}

/// Verify parser output invariants before crossing the WASM boundary.
pub fn sanitize_output(output: SchemaOutput) -> Result<SchemaOutput, SchemaError> {
    match output.detected_format.as_str() {
        "csv" | "json" | "ndjson" => {}
        _ => return Err(SchemaError::UnrecognizedFormat),
    }
    for col in &output.columns {
        sanitize_column(col, output.row_count)?;
    }
    Ok(output)
}

fn sanitize_column(col: &ColumnMeta, row_count: u64) -> Result<(), SchemaError> {
    let err = || SchemaError::CsvParseFailed { row: 0, column: Some(col.index) };

    if col.name.chars().count() > 256 { return Err(err()); }
    if !(0.0..=1.0).contains(&col.null_ratio) || !col.null_ratio.is_finite() { return Err(err()); }
    if col.null_count > row_count { return Err(err()); }

    let is_numeric = matches!(col.inferred_type, InferredType::Integer | InferredType::Float);
    if !is_numeric && (col.numeric_min.is_some() || col.numeric_max.is_some()) { return Err(err()); }

    if col.numeric_min.is_some_and(|v| !v.is_finite()) { return Err(err()); }
    if col.numeric_max.is_some_and(|v| !v.is_finite()) { return Err(err()); }
    if let (Some(min), Some(max)) = (col.numeric_min, col.numeric_max) {
        if min > max { return Err(err()); }
    }

    Ok(())
}

/// Zero-cost marker that infer_schema has no side effects.
/// Enforced structurally by `#![forbid(unsafe_code)]` and the WASM target.
pub struct PurityGuarantee;
impl PurityGuarantee {
    /// No-op call that makes the purity contract visible in the call stack.
    #[inline(always)]
    pub fn assert() {}
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(matches!(validate_input(&big), Err(SchemaError::InputTooLarge { .. })));
    }

    #[test]
    fn nul_byte_rejected() {
        let err = validate_input("name,age\nAlice\x0030").expect_err("NUL must be rejected");
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

    fn make_output(cols: Vec<ColumnMeta>) -> SchemaOutput {
        SchemaOutput {
            row_count: 10, truncated: false,
            detected_format: "csv".to_string(),
            schemasniff_version: "0.1.0".to_string(),
            chunk_count: 1, columns: cols,
        }
    }

    fn make_col(name: &str, t: InferredType) -> ColumnMeta {
        ColumnMeta {
            name: name.to_string(), index: 0, inferred_type: t,
            nullable: false, null_count: 0, null_ratio: 0.0,
            numeric_min: None, numeric_max: None, cardinality_estimate: 0,
        }
    }

    #[test]
    fn clean_output_passes() {
        assert!(sanitize_output(make_output(vec![make_col("age", InferredType::Integer)])).is_ok());
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
        col.null_count = 11;
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
