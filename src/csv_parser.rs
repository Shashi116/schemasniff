//! CSV parser — streams input into per-column accumulators, never storing raw values.

use csv::ReaderBuilder;
use crate::{ColumnMeta, InferredType, SchemaError, SchemaOutput, MAX_CELL_BYTES, MAX_COLS, MAX_ROWS};

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

    /// Observe one cell. Oversized cells are counted as null.
    fn observe(&mut self, raw: &str, _row: usize, _col: usize) -> Result<(), SchemaError> {
        if raw.len() > MAX_CELL_BYTES || raw.is_empty() {
            self.null_count += 1;
            return Ok(());
        }

        self.hll.insert(raw);

        match infer_cell_type(raw) {
            InferredType::Integer => {
                self.votes.integer += 1;
                if let Ok(v) = raw.trim().parse::<i64>() {
                    let fv = v as f64;
                    self.numeric_min = Some(self.numeric_min.map_or(fv, |m| m.min(fv)));
                    self.numeric_max = Some(self.numeric_max.map_or(fv, |m| m.max(fv)));
                }
            }
            InferredType::Float => {
                self.votes.float += 1;
                if let Ok(v) = raw.trim().parse::<f64>() {
                    if v.is_finite() {
                        self.numeric_min = Some(self.numeric_min.map_or(v, |m| m.min(v)));
                        self.numeric_max = Some(self.numeric_max.map_or(v, |m| m.max(v)));
                    }
                }
            }
            InferredType::Boolean => self.votes.boolean += 1,
            InferredType::Date    => self.votes.date    += 1,
            InferredType::String  => self.votes.string  += 1,
            InferredType::Unknown => {}
        }

        Ok(())
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
    /// Majority vote with priority: Integer > Float > Boolean > Date > String.
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

/// Infer the type of a single non-empty cell. No regex, no allocation.
pub(crate) fn infer_cell_type(raw: &str) -> InferredType {
    let s = raw.trim();
    if s.is_empty()    { return InferredType::Unknown; }
    if is_integer(s)   { return InferredType::Integer; }
    if is_float(s)     { return InferredType::Float; }
    if is_boolean(s)   { return InferredType::Boolean; }
    if is_date_like(s) { return InferredType::Date; }
    InferredType::String
}

/// Integers only — rejects leading zeros (IDs, zip codes, etc.).
fn is_integer(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() { return false; }
    let mut chars = digits.chars();
    let first = match chars.next() { Some(c) => c, None => return false };
    if first == '0' && chars.next().is_some() { return false; }
    if !first.is_ascii_digit() { return false; }
    digits.chars().all(|c| c.is_ascii_digit()) && s.parse::<i64>().is_ok()
}

/// Floats must contain `.` or `e`/`E` and parse as finite f64.
fn is_float(s: &str) -> bool {
    if !s.contains('.') && !s.contains('e') && !s.contains('E') { return false; }
    s.parse::<f64>().is_ok_and(|v| v.is_finite())
}

/// Case-insensitive: true, false, yes, no.
fn is_boolean(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no")
}

/// ISO-8601: YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS. Structural check only.
fn is_date_like(s: &str) -> bool {
    if s.len() < 10 { return false; }
    let b = s.as_bytes();
    let date = match b.get(..10) { Some(d) => d, None => return false };

    if !date.get(..4).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }
    if date.get(4) != Some(&b'-') { return false; }
    if !date.get(5..7).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }
    if date.get(7) != Some(&b'-') { return false; }
    if !date.get(8..10).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }

    if s.len() > 10 {
        if s.len() < 19 { return false; }
        let sep = b.get(10);
        if sep != Some(&b'T') && sep != Some(&b' ') { return false; }
        if !b.get(11..13).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }
        if b.get(13) != Some(&b':') { return false; }
        if !b.get(14..16).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }
        if b.get(16) != Some(&b':') { return false; }
        if !b.get(17..19).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }
    }
    true
}

/// Parse a CSV string into a `SchemaOutput`.
pub fn parse_csv(input: &str) -> Result<SchemaOutput, SchemaError> {
    if input.trim().is_empty() { return Err(SchemaError::EmptyInput); }

    // The csv crate drops blank lines — patch them to empty quoted fields.
    let patched: std::borrow::Cow<str> = if input.contains("\n\n") || input.contains("\r\n\r\n") {
        std::borrow::Cow::Owned(
            input.lines()
                .map(|l| if l.trim().is_empty() { "\"\"" } else { l })
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        std::borrow::Cow::Borrowed(input)
    };

    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(patched.as_bytes());

    let headers = reader.headers()
        .map_err(|e| SchemaError::CsvParseFailed {
            row: e.position().map_or(0, |p| p.line() as usize),
            column: None,
        })?
        .clone();

    if headers.is_empty() { return Err(SchemaError::EmptyInput); }

    let col_count = headers.len();
    if col_count > MAX_COLS {
        return Err(SchemaError::TooManyColumns { limit: MAX_COLS, actual: col_count });
    }

    let mut accumulators: Vec<ColumnAccumulator> = headers.iter().enumerate()
        .map(|(i, name)| ColumnAccumulator::new(name.chars().take(256).collect(), i))
        .collect::<Result<Vec<_>, _>>()?;

    let mut row_count: u64 = 0;
    let mut truncated = false;

    for result in reader.records() {
        if row_count as usize >= MAX_ROWS { truncated = true; break; }

        let record = result.map_err(|e| SchemaError::CsvParseFailed {
            row: e.position().map_or(row_count as usize, |p| p.line() as usize),
            column: None,
        })?;

        let effective_cols = record.len().min(col_count);

        for col_idx in 0..effective_cols {
            let cell = record.get(col_idx).unwrap_or("");
            let acc = accumulators.get_mut(col_idx)
                .ok_or(SchemaError::CsvParseFailed { row: row_count as usize, column: Some(col_idx) })?;
            acc.observe(cell, row_count as usize, col_idx)?;
        }

        for col_idx in effective_cols..col_count {
            let acc = accumulators.get_mut(col_idx)
                .ok_or(SchemaError::CsvParseFailed { row: row_count as usize, column: Some(col_idx) })?;
            acc.null_count += 1;
        }

        row_count += 1;
    }

    if row_count == 0 { return Err(SchemaError::EmptyInput); }

    Ok(SchemaOutput {
        row_count, truncated,
        detected_format: "csv".to_string(),
        schemasniff_version: env!("CARGO_PKG_VERSION").to_string(),
        chunk_count: 1,
        columns: accumulators.into_iter().map(|acc| acc.finish(row_count)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_integers() {
        assert_eq!(infer_cell_type("0"),   InferredType::Integer);
        assert_eq!(infer_cell_type("42"),  InferredType::Integer);
        assert_eq!(infer_cell_type("-17"), InferredType::Integer);
        assert_eq!(infer_cell_type("9223372036854775807"), InferredType::Integer);
    }

    #[test]
    fn cell_leading_zero_is_string() {
        assert_eq!(infer_cell_type("007"),  InferredType::String);
        assert_eq!(infer_cell_type("0042"), InferredType::String);
    }

    #[test]
    fn cell_floats() {
        assert_eq!(infer_cell_type("3.14"),   InferredType::Float);
        assert_eq!(infer_cell_type("-0.5"),   InferredType::Float);
        assert_eq!(infer_cell_type("1e10"),   InferredType::Float);
        assert_eq!(infer_cell_type("1.5E-3"), InferredType::Float);
    }

    #[test]
    fn cell_nan_inf_are_string() {
        assert_eq!(infer_cell_type("NaN"),      InferredType::String);
        assert_eq!(infer_cell_type("Infinity"), InferredType::String);
        assert_eq!(infer_cell_type("-Inf"),     InferredType::String);
    }

    #[test]
    fn cell_booleans() {
        assert_eq!(infer_cell_type("true"),  InferredType::Boolean);
        assert_eq!(infer_cell_type("FALSE"), InferredType::Boolean);
        assert_eq!(infer_cell_type("yes"),   InferredType::Boolean);
        assert_eq!(infer_cell_type("No"),    InferredType::Boolean);
    }

    #[test]
    fn cell_dates() {
        assert_eq!(infer_cell_type("2024-01-15"),          InferredType::Date);
        assert_eq!(infer_cell_type("2024-01-15T09:30:00"), InferredType::Date);
        assert_eq!(infer_cell_type("2024-01-15 09:30:00"), InferredType::Date);
        assert_eq!(infer_cell_type("2024-1-1"),            InferredType::String);
        assert_eq!(infer_cell_type("15-01-2024"),          InferredType::String);
    }

    #[test]
    fn cell_empty_is_unknown() {
        assert_eq!(infer_cell_type(""),    InferredType::Unknown);
        assert_eq!(infer_cell_type("   "), InferredType::Unknown);
    }

    #[test]
    fn happy_path_10_rows() {
        let csv = "name,age,score,active,joined\n\
            Alice,30,9.5,true,2023-01-01\nBob,25,8.0,false,2023-02-14\n\
            Carol,40,7.2,true,2023-03-10\nDave,35,6.8,false,2023-04-05\n\
            Eve,28,9.9,true,2023-05-20\nFrank,52,5.5,false,2023-06-30\n\
            Grace,19,8.8,true,2023-07-07\nHeidi,44,7.1,false,2023-08-01\n\
            Ivan,31,6.0,true,2023-09-15\nJudy,27,9.2,false,2023-10-31";
        let out = parse_csv(csv).expect("should parse");
        assert_eq!(out.row_count, 10);
        assert_eq!(out.columns.len(), 5);
        assert_eq!(out.columns[1].inferred_type, InferredType::Integer);
        assert_eq!(out.columns[1].numeric_min, Some(19.0));
        assert_eq!(out.columns[1].numeric_max, Some(52.0));
        assert_eq!(out.columns[2].inferred_type, InferredType::Float);
        assert_eq!(out.columns[3].inferred_type, InferredType::Boolean);
        assert_eq!(out.columns[4].inferred_type, InferredType::Date);
    }

    #[test]
    fn unicode_column_names() {
        let out = parse_csv("名前,年齢\nAlice,30\nBob,25").expect("parse");
        assert_eq!(out.columns[0].name, "名前");
        assert_eq!(out.columns[1].name, "年齢");
    }

    #[test]
    fn column_name_truncated_to_256_chars() {
        let csv = format!("{},age\nAlice,30", "a".repeat(300));
        assert_eq!(parse_csv(&csv).expect("parse").columns[0].name.chars().count(), 256);
    }

    #[test]
    fn empty_input_error() {
        assert!(matches!(parse_csv(""),    Err(SchemaError::EmptyInput)));
        assert!(matches!(parse_csv("   "), Err(SchemaError::EmptyInput)));
    }

    #[test]
    fn header_only_is_empty_input() {
        assert!(matches!(parse_csv("name,age\n"), Err(SchemaError::EmptyInput)));
    }

    #[test]
    fn too_many_columns_error() {
        let headers: Vec<String> = (0..=MAX_COLS).map(|i| format!("col{i}")).collect();
        assert!(matches!(
            parse_csv(&headers.join(",")),
            Err(SchemaError::TooManyColumns { .. })
        ));
    }

    #[test]
    fn oversized_cell_treated_as_null() {
        let csv = format!("value\n{}\nnormal", "x".repeat(MAX_CELL_BYTES + 1));
        let out = parse_csv(&csv).expect("parse");
        assert_eq!(out.columns[0].null_count, 1);
        assert_eq!(out.row_count, 2);
    }

    #[test]
    fn null_ratio_correct() {
        let out = parse_csv("v\nfoo\n\nbar\n\nbaz").expect("parse");
        assert_eq!(out.columns[0].null_count, 2);
        assert!((out.columns[0].null_ratio - 0.4).abs() < 1e-9);
    }

    #[test]
    fn short_rows_padded_as_null() {
        let out = parse_csv("a,b\nfoo,bar\nbaz").expect("parse");
        assert_eq!(out.columns[1].null_count, 1);
    }

    #[test]
    fn numeric_bounds_not_set_for_string_col() {
        let out = parse_csv("name\nAlice\nBob").expect("parse");
        assert_eq!(out.columns[0].numeric_min, None);
        assert_eq!(out.columns[0].numeric_max, None);
    }

    #[test]
    fn truncation_flag_set_at_max_rows() {
        let mut csv = String::from("n\n");
        for i in 0..(MAX_ROWS + 2) { csv.push_str(&format!("{i}\n")); }
        let out = parse_csv(&csv).expect("parse");
        assert!(out.truncated);
        assert_eq!(out.row_count as usize, MAX_ROWS);
    }
}
