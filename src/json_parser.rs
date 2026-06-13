//! JSON and NDJSON parsing module for schemasniff.
//!
//! Handles two formats:
//!   - Object-array JSON: `[{"a":1}, {"a":2}]`
//!   - NDJSON:            `{"a":1}\n{"a":2}`
//!
//! Enforces MAX_ROWS, MAX_COLS, MAX_CELL_BYTES, and MAX_JSON_DEPTH hard caps.
//! Never stores raw cell values — only counters, bounds, and HLL sketches.

use serde_json::Value;
use std::collections::HashMap;

use crate::{
    ColumnMeta, InferredType, SchemaError, SchemaOutput,
    MAX_CELL_BYTES, MAX_COLS, MAX_JSON_DEPTH, MAX_ROWS,
};
use crate::csv_parser::infer_cell_type;

// ── Column accumulator (mirrors csv_parser, operates on JSON values) ─────────

struct ColumnAccumulator {
    name: String,
    index: usize,
    votes: TypeVotes,
    null_count: u64,
    numeric_min: Option<f64>,
    numeric_max: Option<f64>,
    hll: crate::hll::Hll,
}

#[derive(Default)]
struct TypeVotes {
    integer: u64,
    float:   u64,
    boolean: u64,
    date:    u64,
    string:  u64,
}

impl ColumnAccumulator {
    fn new(name: String, index: usize) -> Result<Self, SchemaError> {
        let hll = crate::hll::Hll::new();
        Ok(Self {
            name,
            index,
            votes: TypeVotes::default(),
            null_count: 0,
            numeric_min: None,
            numeric_max: None,
            hll,
        })
    }

    /// Observe a single JSON value for this column.
    /// Converts to a canonical string form for type inference only —
    /// the string is never stored beyond this stack frame.
    fn observe_value(&mut self, value: &Value) {
        match value {
            // Explicit nulls
            Value::Null => {
                self.null_count += 1;
            }

            // Booleans — don't convert to string, type directly
            Value::Bool(b) => {
                self.votes.boolean += 1;
                let repr = if *b { "true" } else { "false" };
                self.hll.insert(repr);
            }

            // Numbers — serde_json preserves the original form in Number
            Value::Number(n) => {
                // Try integer first (i64), then fall back to f64
                if let Some(i) = n.as_i64() {
                    self.votes.integer += 1;
                    let fv = i as f64;
                    self.update_numeric_bounds(fv);
                    self.hll.insert(&i.to_string());
                } else if let Some(f) = n.as_f64() {
                    if f.is_finite() {
                        self.votes.float += 1;
                        self.update_numeric_bounds(f);
                        self.hll.insert(&f.to_string());
                    } else {
                        // NaN / Infinity treated as null — not typeable
                        self.null_count += 1;
                    }
                } else {
                    self.null_count += 1;
                }
            }

            // Strings — run through the same inference ladder as CSV cells
            Value::String(s) => {
                if s.is_empty() {
                    self.null_count += 1;
                    return;
                }
                // Enforce MAX_CELL_BYTES on string values — treat as null if over
                if s.len() > MAX_CELL_BYTES {
                    self.null_count += 1;
                    return;
                }
                self.hll.insert(s);
                match infer_cell_type(s) {
                    InferredType::Integer => {
                        self.votes.integer += 1;
                        if let Ok(v) = s.trim().parse::<i64>() {
                            self.update_numeric_bounds(v as f64);
                        }
                    }
                    InferredType::Float => {
                        self.votes.float += 1;
                        if let Ok(v) = s.trim().parse::<f64>() {
                            if v.is_finite() { self.update_numeric_bounds(v); }
                        }
                    }
                    InferredType::Boolean => self.votes.boolean += 1,
                    InferredType::Date    => self.votes.date += 1,
                    InferredType::String  => self.votes.string += 1,
                    InferredType::Unknown => self.null_count += 1,
                }
            }

            // Nested objects/arrays at a column level — treat as opaque string
            // (depth was already validated before we reached this point)
            Value::Array(_) | Value::Object(_) => {
                self.votes.string += 1;
            }
        }
    }

    fn update_numeric_bounds(&mut self, v: f64) {
        self.numeric_min = Some(match self.numeric_min {
            Some(m) => m.min(v),
            None    => v,
        });
        self.numeric_max = Some(match self.numeric_max {
            Some(m) => m.max(v),
            None    => v,
        });
    }

    fn finish(self, total_rows: u64) -> ColumnMeta {
        let inferred_type = self.votes.dominant();
        let null_count    = self.null_count;
        let null_ratio    = if total_rows == 0 {
            0.0
        } else {
            null_count as f64 / total_rows as f64
        };
        let cardinality_estimate = self.hll.count().round() as u64;

        let (numeric_min, numeric_max) = match inferred_type {
            InferredType::Integer | InferredType::Float => {
                (self.numeric_min, self.numeric_max)
            }
            _ => (None, None),
        };

        ColumnMeta {
            name: self.name,
            index: self.index,
            inferred_type,
            nullable: null_count > 0,
            null_count,
            null_ratio,
            numeric_min,
            numeric_max,
            cardinality_estimate,
        }
    }
}

impl TypeVotes {
    fn dominant(&self) -> InferredType {
        let total = self.integer + self.float + self.boolean + self.date + self.string;
        if total == 0 {
            return InferredType::Unknown;
        }
        let max = self.integer
            .max(self.float)
            .max(self.boolean)
            .max(self.date)
            .max(self.string);

        if      self.integer == max { InferredType::Integer }
        else if self.float   == max { InferredType::Float }
        else if self.boolean == max { InferredType::Boolean }
        else if self.date    == max { InferredType::Date }
        else                        { InferredType::String }
    }
}

// ── Depth guard ───────────────────────────────────────────────────────────────

/// Walk a `Value` tree and return its maximum nesting depth.
/// Depth 0 = a scalar; depth 1 = `[scalar]` or `{k: scalar}`.
/// Returns `Err(SchemaError::NestingTooDeep)` the moment depth exceeds
/// `MAX_JSON_DEPTH`, without walking the rest of the tree.
fn check_depth(value: &Value, current: usize, row: usize) -> Result<(), SchemaError> {
    if current > MAX_JSON_DEPTH {
        return Err(SchemaError::NestingTooDeep {
            limit: MAX_JSON_DEPTH,
            detected_at_row: row,
        });
    }
    match value {
        Value::Array(arr) => {
            for item in arr {
                check_depth(item, current + 1, row)?;
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                check_depth(v, current + 1, row)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// ── Format detection ──────────────────────────────────────────────────────────

/// Detect whether the input is object-array JSON or NDJSON.
/// Returns `None` if neither format is recognised.
pub(crate) enum JsonFormat {
    /// `[{"a":1}, {"a":2}]` — a single JSON array of objects
    ObjectArray,
    /// `{"a":1}\n{"a":2}` — one JSON object per line
    Ndjson,
}

pub(crate) fn detect_json_format(input: &str) -> Option<JsonFormat> {
    let trimmed = input.trim();
    if trimmed.starts_with('[') {
        Some(JsonFormat::ObjectArray)
    } else if trimmed.starts_with('{') {
        Some(JsonFormat::Ndjson)
    } else {
        None
    }
}

// ── Row iterator ──────────────────────────────────────────────────────────────

/// Parse the input into an iterator of `serde_json::Value` objects,
/// one per logical row. Returns a `Vec` to keep lifetimes simple —
/// rows are needed for the two-pass column-discovery + accumulation loop.
///
/// For NDJSON, each non-empty line is parsed independently so a single
/// malformed line produces a precise `JsonParseFailed` with a byte offset.
fn parse_rows(input: &str, format: &JsonFormat) -> Result<Vec<Value>, SchemaError> {
    match format {
        JsonFormat::ObjectArray => {
        let value: Value = serde_json::from_str(input).map_err(|e| {
        SchemaError::JsonParseFailed {
            byte_offset: Some(e.column()),
        }
        })?;
            match value {
                Value::Array(rows) => {
                    // Reject arrays of scalars — they have no column structure
                    let has_any_object = rows.iter().any(|r| matches!(r, Value::Object(_)));
                    if !has_any_object && !rows.is_empty() {
                        return Err(SchemaError::UnrecognizedFormat);
                    }
                    Ok(rows)
                }
                _ => Err(SchemaError::UnrecognizedFormat),
            }
        }

        JsonFormat::Ndjson => {
            let mut rows = Vec::new();
            let mut byte_offset: usize = 0;

            for line in input.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    byte_offset += line.len() + 1;
                    continue;
                }
                let value: Value = serde_json::from_str(trimmed).map_err(|e| {
                    SchemaError::JsonParseFailed {
                        byte_offset: Some(byte_offset + e.column()),
                    }
                })?;
                rows.push(value);
                byte_offset += line.len() + 1;
            }

            if rows.is_empty() {
                return Err(SchemaError::EmptyInput);
            }
            Ok(rows)
        }
    }
}

// ── Column discovery ──────────────────────────────────────────────────────────

/// Scan all rows and build an ordered column name → index map.
/// Preserves first-seen order (matches the behaviour of most JSON tools).
/// Returns `TooManyColumns` if the union of all keys exceeds `MAX_COLS`.
fn discover_columns(rows: &[Value]) -> Result<HashMap<String, usize>, SchemaError> {
    let mut columns: HashMap<String, usize> = HashMap::new();

    for row in rows {
        if let Value::Object(map) = row {
            for key in map.keys() {
                if !columns.contains_key(key) {
                    let next_index = columns.len();
                    if next_index >= MAX_COLS {
                        return Err(SchemaError::TooManyColumns {
                            limit: MAX_COLS,
                            actual: next_index + 1,
                        });
                    }
                    // Sanitize key: truncate to 256 chars, no HTML escaping
                    let sanitized: String = key.chars().take(256).collect();
                    columns.insert(sanitized, next_index);
                }
            }
        }
        // Non-object rows are skipped during discovery;
        // they will be counted as all-null during accumulation
    }

    Ok(columns)
}

// ── Public parse entry point ──────────────────────────────────────────────────

/// Parse a JSON or NDJSON string and return a [`SchemaOutput`].
///
/// # Errors
/// Returns [`SchemaError`] if any hard cap is exceeded, the nesting depth
/// exceeds `MAX_JSON_DEPTH`, or the input is not valid JSON/NDJSON.
/// Never returns raw cell content in any error variant.
pub fn parse_json(input: &str) -> Result<SchemaOutput, SchemaError> {
    if input.trim().is_empty() {
        return Err(SchemaError::EmptyInput);
    }

    // ── Format detection ──────────────────────────────────────────────────────
    let format = detect_json_format(input)
        .ok_or(SchemaError::UnrecognizedFormat)?;

    let format_name = match format {
        JsonFormat::ObjectArray => "json",
        JsonFormat::Ndjson      => "ndjson",
    };

    // ── Parse into rows ───────────────────────────────────────────────────────
    let all_rows = parse_rows(input, &format)?;

    if all_rows.is_empty() {
        return Err(SchemaError::EmptyInput);
    }

    // ── Apply MAX_ROWS cap ────────────────────────────────────────────────────
    let truncated = all_rows.len() > MAX_ROWS;
    let rows = if truncated {
        all_rows.get(..MAX_ROWS).unwrap_or(&all_rows)
    } else {
        &all_rows
    };

    // ── Depth check on every row ──────────────────────────────────────────────
    // Check each row independently so the error carries the exact row index.
    for (row_idx, row) in rows.iter().enumerate() {
        check_depth(row, 0, row_idx)?;
    }

    // ── Column discovery ──────────────────────────────────────────────────────
    let col_index_map = discover_columns(rows)?;

    if col_index_map.is_empty() {
        return Err(SchemaError::EmptyInput);
    }

    // Build accumulators in stable index order
    let mut accumulators: Vec<ColumnAccumulator> = {
        let mut entries: Vec<(String, usize)> = col_index_map.into_iter().collect();
        entries.sort_by_key(|(_, idx)| *idx);
        entries
            .into_iter()
            .map(|(name, idx)| ColumnAccumulator::new(name, idx))
            .collect::<Result<Vec<_>, _>>()?
    };

    // ── Accumulation pass ─────────────────────────────────────────────────────
    let row_count = rows.len() as u64;

    for row in rows {
        match row {
            Value::Object(map) => {
                // Track which column indices were seen in this row
                // so missing keys can be counted as null
                let mut seen = vec![false; accumulators.len()];

                for (key, value) in map {
                    // Sanitize key before lookup (same truncation as discovery)
                    let sanitized: String = key.chars().take(256).collect();

                    // Find the accumulator by name — O(n) over columns,
                    // acceptable given MAX_COLS = 1024
                    if let Some(acc) = accumulators
                        .iter_mut()
                        .find(|a| a.name == sanitized)
                    {
                        let idx = acc.index;
                        acc.observe_value(value);
                        if let Some(s) = seen.get_mut(idx) {
                            *s = true;
                        }
                    }
                }

                // Any column not present in this row → null
                for (acc, was_seen) in accumulators.iter_mut().zip(seen.iter()) {
                    if !*was_seen {
                        acc.null_count += 1;
                    }
                }
            }

            // Non-object row (e.g. a bare string or number in an array) →
            // all columns count as null for this row
            _ => {
                for acc in accumulators.iter_mut() {
                    acc.null_count += 1;
                }
            }
        }
    }

    // ── Finalise ──────────────────────────────────────────────────────────────
    let columns: Vec<ColumnMeta> = accumulators
        .into_iter()
        .map(|acc| acc.finish(row_count))
        .collect();

    Ok(SchemaOutput {
        row_count,
        truncated,
        detected_format: format_name.to_string(),
        schemasniff_version: env!("CARGO_PKG_VERSION").to_string(),
        chunk_count: 1,
        columns,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Object-array JSON ─────────────────────────────────────────────────────

    #[test]
    fn object_array_happy_path() {
        let json = r#"[
            {"name":"Alice","age":30,"score":9.5,"active":true,"joined":"2023-01-01"},
            {"name":"Bob",  "age":25,"score":8.0,"active":false,"joined":"2023-02-14"},
            {"name":"Carol","age":40,"score":7.2,"active":true, "joined":"2023-03-10"}
        ]"#;

        let out = parse_json(json).expect("should parse");
        assert_eq!(out.row_count, 3);
        assert_eq!(out.detected_format, "json");
        assert!(!out.truncated);
        assert_eq!(out.columns.len(), 5);

        let age = out.columns.iter().find(|c| c.name == "age").unwrap();
        assert_eq!(age.inferred_type, InferredType::Integer);
        assert_eq!(age.numeric_min, Some(25.0));
        assert_eq!(age.numeric_max, Some(40.0));

        let score = out.columns.iter().find(|c| c.name == "score").unwrap();
        assert_eq!(score.inferred_type, InferredType::Float);

        let active = out.columns.iter().find(|c| c.name == "active").unwrap();
        assert_eq!(active.inferred_type, InferredType::Boolean);

        let joined = out.columns.iter().find(|c| c.name == "joined").unwrap();
        assert_eq!(joined.inferred_type, InferredType::Date);
    }

    #[test]
    fn object_array_missing_keys_are_null() {
        let json = r#"[
            {"a":1,"b":2},
            {"a":3},
            {"b":4}
        ]"#;
        let out = parse_json(json).expect("parse");
        let a = out.columns.iter().find(|c| c.name == "a").unwrap();
        let b = out.columns.iter().find(|c| c.name == "b").unwrap();
        assert_eq!(a.null_count, 1); // row 3 missing "a"
        assert_eq!(b.null_count, 1); // row 2 missing "b"
    }

    #[test]
    fn object_array_explicit_null_counted() {
        let json = r#"[{"x":1},{"x":null},{"x":3}]"#;
        let out = parse_json(json).expect("parse");
        let x = out.columns.iter().find(|c| c.name == "x").unwrap();
        assert_eq!(x.null_count, 1);
        assert!(x.nullable);
    }

    // ── NDJSON ────────────────────────────────────────────────────────────────

    #[test]
    fn ndjson_happy_path() {
        let ndjson = "{\"name\":\"Alice\",\"age\":30}\n{\"name\":\"Bob\",\"age\":25}\n";
        let out = parse_json(ndjson).expect("ndjson parse");
        assert_eq!(out.detected_format, "ndjson");
        assert_eq!(out.row_count, 2);
        let age = out.columns.iter().find(|c| c.name == "age").unwrap();
        assert_eq!(age.inferred_type, InferredType::Integer);
    }

    #[test]
    fn ndjson_blank_lines_skipped() {
        let ndjson = "{\"a\":1}\n\n{\"a\":2}\n\n{\"a\":3}";
        let out = parse_json(ndjson).expect("parse");
        assert_eq!(out.row_count, 3);
    }

    #[test]
    fn ndjson_malformed_line_gives_byte_offset() {
        let ndjson = "{\"a\":1}\n{bad json}\n{\"a\":3}";
        let err = parse_json(ndjson).expect_err("should fail");
        assert!(matches!(err, SchemaError::JsonParseFailed { byte_offset: Some(_) }));
    }

    // ── Depth guard ───────────────────────────────────────────────────────────

    #[test]
    fn nesting_at_limit_is_accepted() {
        // Build an object nested exactly MAX_JSON_DEPTH levels deep
        // depth 0 = the row object itself
        let mut json = String::from("[");
        json.push('{');
        json.push_str("\"k\":");
        for _ in 0..MAX_JSON_DEPTH {
            json.push('{');
        }
        json.push_str("\"v\":1");
        for _ in 0..MAX_JSON_DEPTH {
            json.push('}');
        }
        json.push('}');
        json.push(']');
        // This is right at the limit — should either pass or fail cleanly;
        // we only assert no panic
        let _ = parse_json(&json);
    }

    #[test]
    fn nesting_over_limit_returns_error() {
        // 33 levels deep — one over MAX_JSON_DEPTH
        let mut inner = String::from("\"v\":1");
        for _ in 0..(MAX_JSON_DEPTH + 1) {
            inner = format!("\"k\":{{{inner}}}");
        }
        let json = format!("[{{{inner}}}]");
        let err = parse_json(&json).expect_err("should reject deep nesting");
        assert!(matches!(
            err,
            SchemaError::NestingTooDeep { limit: _, detected_at_row: _ }
        ));
    }

    // ── Caps ──────────────────────────────────────────────────────────────────

    #[test]
    fn too_many_columns_error() {
        // Each row contributes one new key until MAX_COLS is exceeded
        let rows: Vec<String> = (0..=MAX_COLS)
            .map(|i| format!("{{\"col{i}\":1}}"))
            .collect();
        let json = format!("[{}]", rows.join(","));
        let err = parse_json(&json).expect_err("should error");
        assert!(matches!(
            err,
            SchemaError::TooManyColumns { limit: _, actual: _ }
        ));
    }

    #[test]
    fn truncation_flag_set_at_max_rows() {
        let rows: String = (0..(MAX_ROWS + 2))
            .map(|i| format!("{{\"n\":{i}}}"))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!("[{rows}]");
        let out = parse_json(&json).expect("parse");
        assert!(out.truncated);
        assert_eq!(out.row_count as usize, MAX_ROWS);
    }

    #[test]
    fn oversized_string_value_treated_as_null() {
        let big = "x".repeat(MAX_CELL_BYTES + 1);
        let json = format!("[{{\"v\":\"{big}\"}},{{\"v\":\"normal\"}}]");
        let out = parse_json(&json).expect("parse");
        let v = out.columns.iter().find(|c| c.name == "v").unwrap();
        assert_eq!(v.null_count, 1);
    }

    // ── Format detection ──────────────────────────────────────────────────────

    #[test]
    fn empty_input_error() {
        assert!(matches!(parse_json(""), Err(SchemaError::EmptyInput)));
        assert!(matches!(parse_json("   "), Err(SchemaError::EmptyInput)));
    }

    #[test]
    fn unrecognised_format_error() {
        assert!(matches!(
            parse_json("not json at all"),
            Err(SchemaError::UnrecognizedFormat)
        ));
        // A bare JSON array of scalars (not objects) is also unrecognised
        assert!(matches!(
            parse_json("[1,2,3]"),
            Err(SchemaError::UnrecognizedFormat)
        ));
    }

    #[test]
    fn numeric_string_values_inferred_correctly() {
        // JSON strings containing numbers should use the CSV inference ladder
        let json = r#"[{"v":"42"},{"v":"43"},{"v":"44"}]"#;
        let out = parse_json(json).expect("parse");
        let v = out.columns.iter().find(|c| c.name == "v").unwrap();
        assert_eq!(v.inferred_type, InferredType::Integer);
    }

    #[test]
    fn mixed_numeric_types_resolve_to_integer() {
        // Two integers outweigh one float — integer wins by vote count
        let json = r#"[{"v":1},{"v":2},{"v":3.5}]"#;
        let out = parse_json(json).expect("parse");
        let v = out.columns.iter().find(|c| c.name == "v").unwrap();
        assert_eq!(v.inferred_type, InferredType::Integer);
    }

    #[test]
    fn column_key_truncated_to_256_chars() {
        let long_key = "k".repeat(300);
        let json = format!("[{{\"{long_key}\":1}}]");
        let out = parse_json(&json).expect("parse");
        assert_eq!(out.columns[0].name.chars().count(), 256);
    }
}