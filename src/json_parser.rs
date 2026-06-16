//! JSON and NDJSON parser — object-array `[{...}]` and line-delimited `{...}\n{...}`.

use serde_json::Value;
use std::collections::HashMap;
use crate::{ColumnMeta, InferredType, SchemaError, SchemaOutput, MAX_CELL_BYTES, MAX_COLS, MAX_JSON_DEPTH, MAX_ROWS};
use crate::csv_parser::infer_cell_type;

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
        Ok(Self {
            name, index,
            votes: TypeVotes::default(),
            null_count: 0,
            numeric_min: None,
            numeric_max: None,
            hll: crate::hll::Hll::new(),
        })
    }

    fn observe_value(&mut self, value: &Value) {
        match value {
            Value::Null => self.null_count += 1,

            Value::Bool(b) => {
                self.votes.boolean += 1;
                self.hll.insert(if *b { "true" } else { "false" });
            }

            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    self.votes.integer += 1;
                    self.update_bounds(i as f64);
                    self.hll.insert(&i.to_string());
                } else if let Some(f) = n.as_f64() {
                    if f.is_finite() {
                        self.votes.float += 1;
                        self.update_bounds(f);
                        self.hll.insert(&f.to_string());
                    } else {
                        self.null_count += 1;
                    }
                } else {
                    self.null_count += 1;
                }
            }

            Value::String(s) => {
                if s.is_empty() || s.len() > MAX_CELL_BYTES {
                    self.null_count += 1;
                    return;
                }
                self.hll.insert(s);
                match infer_cell_type(s) {
                    InferredType::Integer => {
                        self.votes.integer += 1;
                        if let Ok(v) = s.trim().parse::<i64>() { self.update_bounds(v as f64); }
                    }
                    InferredType::Float => {
                        self.votes.float += 1;
                        if let Ok(v) = s.trim().parse::<f64>() {
                            if v.is_finite() { self.update_bounds(v); }
                        }
                    }
                    InferredType::Boolean => self.votes.boolean += 1,
                    InferredType::Date    => self.votes.date    += 1,
                    InferredType::String  => self.votes.string  += 1,
                    InferredType::Unknown => self.null_count    += 1,
                }
            }

            // Nested objects/arrays — treat as opaque string
            Value::Array(_) | Value::Object(_) => self.votes.string += 1,
        }
    }

    fn update_bounds(&mut self, v: f64) {
        self.numeric_min = Some(self.numeric_min.map_or(v, |m| m.min(v)));
        self.numeric_max = Some(self.numeric_max.map_or(v, |m| m.max(v)));
    }

    fn finish(self, total_rows: u64) -> ColumnMeta {
        let inferred_type = self.votes.dominant();
        let null_ratio = if total_rows == 0 { 0.0 } else { self.null_count as f64 / total_rows as f64 };
        let (numeric_min, numeric_max) = match inferred_type {
            InferredType::Integer | InferredType::Float => (self.numeric_min, self.numeric_max),
            _ => (None, None),
        };
        ColumnMeta {
            name: self.name, index: self.index, inferred_type,
            nullable: self.null_count > 0, null_count: self.null_count, null_ratio,
            numeric_min, numeric_max,
            cardinality_estimate: self.hll.count().round() as u64,
        }
    }
}

impl TypeVotes {
    fn dominant(&self) -> InferredType {
        let total = self.integer + self.float + self.boolean + self.date + self.string;
        if total == 0 { return InferredType::Unknown; }
        let max = self.integer.max(self.float).max(self.boolean).max(self.date).max(self.string);
        if      self.integer == max { InferredType::Integer }
        else if self.float   == max { InferredType::Float }
        else if self.boolean == max { InferredType::Boolean }
        else if self.date    == max { InferredType::Date }
        else                        { InferredType::String }
    }
}

fn check_depth(value: &Value, current: usize, row: usize) -> Result<(), SchemaError> {
    if current > MAX_JSON_DEPTH {
        return Err(SchemaError::NestingTooDeep { limit: MAX_JSON_DEPTH, detected_at_row: row });
    }
    match value {
        Value::Array(arr)  => { for item in arr  { check_depth(item, current + 1, row)?; } }
        Value::Object(map) => { for v in map.values() { check_depth(v, current + 1, row)?; } }
        _ => {}
    }
    Ok(())
}

pub(crate) enum JsonFormat { ObjectArray, Ndjson }

pub(crate) fn detect_json_format(input: &str) -> Option<JsonFormat> {
    let t = input.trim();
    if t.starts_with('[')      { Some(JsonFormat::ObjectArray) }
    else if t.starts_with('{') { Some(JsonFormat::Ndjson) }
    else                       { None }
}

fn parse_rows(input: &str, format: &JsonFormat) -> Result<Vec<Value>, SchemaError> {
    match format {
        JsonFormat::ObjectArray => {
            let value: Value = serde_json::from_str(input)
                .map_err(|e| SchemaError::JsonParseFailed { byte_offset: Some(e.column()) })?;
            match value {
                Value::Array(rows) => {
                    if !rows.is_empty() && !rows.iter().any(|r| matches!(r, Value::Object(_))) {
                        return Err(SchemaError::UnrecognizedFormat);
                    }
                    Ok(rows)
                }
                _ => Err(SchemaError::UnrecognizedFormat),
            }
        }
        JsonFormat::Ndjson => {
            let mut rows = Vec::new();
            let mut offset: usize = 0;
            for line in input.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() { offset += line.len() + 1; continue; }
                let value: Value = serde_json::from_str(trimmed)
                    .map_err(|e| SchemaError::JsonParseFailed { byte_offset: Some(offset + e.column()) })?;
                rows.push(value);
                offset += line.len() + 1;
            }
            if rows.is_empty() { return Err(SchemaError::EmptyInput); }
            Ok(rows)
        }
    }
}

fn discover_columns(rows: &[Value]) -> Result<HashMap<String, usize>, SchemaError> {
    let mut columns: HashMap<String, usize> = HashMap::new();
    for row in rows {
        if let Value::Object(map) = row {
            for key in map.keys() {
                if !columns.contains_key(key) {
                    let idx = columns.len();
                    if idx >= MAX_COLS {
                        return Err(SchemaError::TooManyColumns { limit: MAX_COLS, actual: idx + 1 });
                    }
                    columns.insert(key.chars().take(256).collect(), idx);
                }
            }
        }
    }
    Ok(columns)
}

/// Parse a JSON or NDJSON string into a `SchemaOutput`.
pub fn parse_json(input: &str) -> Result<SchemaOutput, SchemaError> {
    if input.trim().is_empty() { return Err(SchemaError::EmptyInput); }

    let format = detect_json_format(input).ok_or(SchemaError::UnrecognizedFormat)?;
    let format_name = match format { JsonFormat::ObjectArray => "json", JsonFormat::Ndjson => "ndjson" };

    let all_rows = parse_rows(input, &format)?;
    if all_rows.is_empty() { return Err(SchemaError::EmptyInput); }

    let truncated = all_rows.len() > MAX_ROWS;
    let rows = if truncated { all_rows.get(..MAX_ROWS).unwrap_or(&all_rows) } else { &all_rows };

    for (i, row) in rows.iter().enumerate() { check_depth(row, 0, i)?; }

    let col_index_map = discover_columns(rows)?;
    if col_index_map.is_empty() { return Err(SchemaError::EmptyInput); }

    let mut accumulators: Vec<ColumnAccumulator> = {
        let mut entries: Vec<(String, usize)> = col_index_map.into_iter().collect();
        entries.sort_by_key(|(_, idx)| *idx);
        entries.into_iter()
            .map(|(name, idx)| ColumnAccumulator::new(name, idx))
            .collect::<Result<Vec<_>, _>>()?
    };

    let row_count = rows.len() as u64;

    for row in rows {
        match row {
            Value::Object(map) => {
                let mut seen = vec![false; accumulators.len()];
                for (key, value) in map {
                    let sanitized: String = key.chars().take(256).collect();
                    if let Some(acc) = accumulators.iter_mut().find(|a| a.name == sanitized) {
                        let idx = acc.index;
                        acc.observe_value(value);
                        if let Some(s) = seen.get_mut(idx) { *s = true; }
                    }
                }
                for (acc, was_seen) in accumulators.iter_mut().zip(seen.iter()) {
                    if !*was_seen { acc.null_count += 1; }
                }
            }
            _ => { for acc in accumulators.iter_mut() { acc.null_count += 1; } }
        }
    }

    Ok(SchemaOutput {
        row_count, truncated,
        detected_format: format_name.to_string(),
        schemasniff_version: env!("CARGO_PKG_VERSION").to_string(),
        chunk_count: 1,
        columns: accumulators.into_iter().map(|acc| acc.finish(row_count)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_array_happy_path() {
        let json = r#"[
            {"name":"Alice","age":30,"score":9.5,"active":true,"joined":"2023-01-01"},
            {"name":"Bob",  "age":25,"score":8.0,"active":false,"joined":"2023-02-14"},
            {"name":"Carol","age":40,"score":7.2,"active":true, "joined":"2023-03-10"}
        ]"#;
        let out = parse_json(json).expect("parse");
        assert_eq!(out.row_count, 3);
        assert_eq!(out.detected_format, "json");
        assert_eq!(out.columns.len(), 5);
        let age = out.columns.iter().find(|c| c.name == "age").unwrap();
        assert_eq!(age.inferred_type, InferredType::Integer);
        assert_eq!(age.numeric_min, Some(25.0));
        assert_eq!(age.numeric_max, Some(40.0));
    }

    #[test]
    fn object_array_missing_keys_are_null() {
        let json = r#"[{"a":1,"b":2},{"a":3},{"b":4}]"#;
        let out = parse_json(json).expect("parse");
        assert_eq!(out.columns.iter().find(|c| c.name == "a").unwrap().null_count, 1);
        assert_eq!(out.columns.iter().find(|c| c.name == "b").unwrap().null_count, 1);
    }

    #[test]
    fn object_array_explicit_null_counted() {
        let json = r#"[{"x":1},{"x":null},{"x":3}]"#;
        let out = parse_json(json).expect("parse");
        let x = out.columns.iter().find(|c| c.name == "x").unwrap();
        assert_eq!(x.null_count, 1);
        assert!(x.nullable);
    }

    #[test]
    fn ndjson_happy_path() {
        let ndjson = "{\"name\":\"Alice\",\"age\":30}\n{\"name\":\"Bob\",\"age\":25}\n";
        let out = parse_json(ndjson).expect("parse");
        assert_eq!(out.detected_format, "ndjson");
        assert_eq!(out.row_count, 2);
        assert_eq!(out.columns.iter().find(|c| c.name == "age").unwrap().inferred_type, InferredType::Integer);
    }

    #[test]
    fn ndjson_blank_lines_skipped() {
        let out = parse_json("{\"a\":1}\n\n{\"a\":2}\n\n{\"a\":3}").expect("parse");
        assert_eq!(out.row_count, 3);
    }

    #[test]
    fn ndjson_malformed_line_gives_byte_offset() {
        let err = parse_json("{\"a\":1}\n{bad json}\n{\"a\":3}").expect_err("fail");
        assert!(matches!(err, SchemaError::JsonParseFailed { byte_offset: Some(_) }));
    }

    #[test]
    fn nesting_at_limit_is_accepted() {
        let mut json = String::from("[{\"k\":");
        for _ in 0..MAX_JSON_DEPTH { json.push('{'); }
        json.push_str("\"v\":1");
        for _ in 0..MAX_JSON_DEPTH { json.push('}'); }
        json.push_str("}]");
        let _ = parse_json(&json);
    }

    #[test]
    fn nesting_over_limit_returns_error() {
        let mut inner = String::from("\"v\":1");
        for _ in 0..(MAX_JSON_DEPTH + 1) { inner = format!("\"k\":{{{inner}}}"); }
        let err = parse_json(&format!("[{{{inner}}}]")).expect_err("fail");
        assert!(matches!(err, SchemaError::NestingTooDeep { .. }));
    }

    #[test]
    fn too_many_columns_error() {
        let rows: Vec<String> = (0..=MAX_COLS).map(|i| format!("{{\"col{i}\":1}}")).collect();
        assert!(matches!(
            parse_json(&format!("[{}]", rows.join(","))),
            Err(SchemaError::TooManyColumns { .. })
        ));
    }

    #[test]
    fn truncation_flag_set_at_max_rows() {
        let rows = (0..(MAX_ROWS + 2)).map(|i| format!("{{\"n\":{i}}}")).collect::<Vec<_>>().join(",");
        let out = parse_json(&format!("[{rows}]")).expect("parse");
        assert!(out.truncated);
        assert_eq!(out.row_count as usize, MAX_ROWS);
    }

    #[test]
    fn oversized_string_value_treated_as_null() {
        let big = "x".repeat(MAX_CELL_BYTES + 1);
        let json = format!("[{{\"v\":\"{big}\"}},{{\"v\":\"normal\"}}]");
        let out = parse_json(&json).expect("parse");
        assert_eq!(out.columns.iter().find(|c| c.name == "v").unwrap().null_count, 1);
    }

    #[test]
    fn empty_input_error() {
        assert!(matches!(parse_json(""),    Err(SchemaError::EmptyInput)));
        assert!(matches!(parse_json("   "), Err(SchemaError::EmptyInput)));
    }

    #[test]
    fn unrecognised_format_error() {
        assert!(matches!(parse_json("not json"), Err(SchemaError::UnrecognizedFormat)));
        assert!(matches!(parse_json("[1,2,3]"),   Err(SchemaError::UnrecognizedFormat)));
    }

    #[test]
    fn numeric_string_values_inferred_correctly() {
        let json = r#"[{"v":"42"},{"v":"43"},{"v":"44"}]"#;
        let out = parse_json(json).expect("parse");
        assert_eq!(out.columns.iter().find(|c| c.name == "v").unwrap().inferred_type, InferredType::Integer);
    }

    #[test]
    fn mixed_numeric_types_resolve_to_integer() {
        let json = r#"[{"v":1},{"v":2},{"v":3.5}]"#;
        let out = parse_json(json).expect("parse");
        assert_eq!(out.columns.iter().find(|c| c.name == "v").unwrap().inferred_type, InferredType::Integer);
    }

    #[test]
    fn column_key_truncated_to_256_chars() {
        let json = format!("[{{\"{}\":1}}]", "k".repeat(300));
        assert_eq!(parse_json(&json).expect("parse").columns[0].name.chars().count(), 256);
    }
}
