//! CSV parsing module for schemasniff.
//!
//! Parses a UTF-8 CSV string into a [`SchemaOutput`] using hard security caps
//! defined in the crate root. No heap allocation beyond what the output structs
//! require; no raw cell values are ever stored or returned.

use csv::ReaderBuilder;
use crate::{
    ColumnMeta, InferredType, SchemaError, SchemaOutput,
    MAX_CELL_BYTES, MAX_COLS, MAX_ROWS,
};

// ── Column accumulator ────────────────────────────────────────────────────────

/// Accumulates per-column statistics during a single streaming pass.
/// Never stores raw cell values — only counters, bounds, and a HLL sketch.
struct ColumnAccumulator {
    name: String,
    index: usize,
    /// Running vote tallies for type inference
    votes: TypeVotes,
    null_count: u64,
    numeric_min: Option<f64>,
    numeric_max: Option<f64>,
    /// HyperLogLog++ sketch — inserts a hash of each cell, never the cell itself
    hll: crate::hll::Hll,
}

/// Vote counts for each candidate type — whichever wins becomes InferredType.
#[derive(Default)]
struct TypeVotes {
    integer: u64,
    float: u64,
    boolean: u64,
    date: u64,
    string: u64,
}

impl ColumnAccumulator {
    fn new(name: String, index: usize) -> Result<Self, SchemaError> {
        // precision 14 → ±0.8% error, ~16 KB per sketch — acceptable for 1024 cols
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

    /// Record a single cell. Enforces MAX_CELL_BYTES by treating oversized
    /// cells as null — they cannot be typed reliably and storing them risks OOM.
    fn observe(&mut self, raw: &str, _row: usize, _col: usize) -> Result<(), SchemaError> {
        if raw.len() > MAX_CELL_BYTES {
            // Treat as null — do not error, do not store, just count
            self.null_count += 1;
            return Ok(());
        }

        if raw.is_empty() {
            self.null_count += 1;
            return Ok(());
        }

        // Insert a hash of the value — the raw string never lives past this frame
        self.hll.insert(raw);

        // Type inference: deterministic priority ladder, no regex
        match infer_cell_type(raw) {
            InferredType::Integer => {
                self.votes.integer += 1;
                // Safe: we already confirmed it parses as i64 inside infer_cell_type
                if let Ok(v) = raw.trim().parse::<i64>() {
                    let fv = v as f64;
                    self.numeric_min = Some(match self.numeric_min {
                        Some(m) => m.min(fv),
                        None => fv,
                    });
                    self.numeric_max = Some(match self.numeric_max {
                        Some(m) => m.max(fv),
                        None => fv,
                    });
                }
            }
            InferredType::Float => {
                self.votes.float += 1;
                if let Ok(v) = raw.trim().parse::<f64>() {
                    if v.is_finite() {
                        self.numeric_min = Some(match self.numeric_min {
                            Some(m) => m.min(v),
                            None => v,
                        });
                        self.numeric_max = Some(match self.numeric_max {
                            Some(m) => m.max(v),
                            None => v,
                        });
                    }
                }
            }
            InferredType::Boolean => self.votes.boolean += 1,
            InferredType::Date    => self.votes.date += 1,
            InferredType::String  => self.votes.string += 1,
            InferredType::Unknown => {} // null already counted above
        }

        Ok(())
    }

    /// Finalise into a [`ColumnMeta`]. Consumes the accumulator.
    fn finish(self, total_rows: u64) -> ColumnMeta {
        let inferred_type = self.votes.dominant();
        let null_count = self.null_count;
        let null_ratio = if total_rows == 0 {
            0.0
        } else {
            null_count as f64 / total_rows as f64
        };
        let cardinality_estimate = self.hll.count().round() as u64;

        // Only expose numeric bounds for numeric columns
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
    /// Return the winning type by strict priority.
    /// Priority: Integer > Float > Boolean > Date > String > Unknown.
    /// Ties are broken by priority order — a column with equal integer and
    /// float votes is called Integer, which is the stricter type.
    fn dominant(&self) -> InferredType {
        let total = self.integer + self.float + self.boolean + self.date + self.string;
        if total == 0 {
            return InferredType::Unknown;
        }
        // A column is typed by its majority vote winner, with the priority
        // ladder as a tiebreaker. This prevents a single stray "true" from
        // overriding 999 numeric values in a near-homogeneous column.
        let max = self.integer
            .max(self.float)
            .max(self.boolean)
            .max(self.date)
            .max(self.string);

        if self.integer == max { InferredType::Integer }
        else if self.float   == max { InferredType::Float }
        else if self.boolean == max { InferredType::Boolean }
        else if self.date    == max { InferredType::Date }
        else                        { InferredType::String }
    }
}

// ── Cell type inference ───────────────────────────────────────────────────────

/// Infer the type of a single non-empty, within-size cell.
/// Deterministic priority ladder — no regex, no allocation.
pub(crate) fn infer_cell_type(raw: &str) -> InferredType {
    let s = raw.trim();

    if s.is_empty() {
        return InferredType::Unknown;
    }

    // i64 — strict integer, no leading zeros (which would be ID strings)
    if is_integer(s) {
        return InferredType::Integer;
    }

    // f64 — decimal point or exponent present
    if is_float(s) {
        return InferredType::Float;
    }

    // bool — case-insensitive, exactly these four tokens
    if is_boolean(s) {
        return InferredType::Boolean;
    }

    // date-like — structural check only, no regex
    if is_date_like(s) {
        return InferredType::Date;
    }

    InferredType::String
}

/// Returns true for bare integers. Rejects leading zeros ("007", "01") because
/// those are almost always identifier strings, not numbers.
fn is_integer(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() {
        return false;
    }
    // Reject leading zeros on multi-digit numbers
    let mut chars = digits.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if first == '0' && chars.next().is_some() {
        return false; // leading zero
    }
    // All remaining chars must be ASCII digits
    if !first.is_ascii_digit() {
        return false;
    }
    digits.chars().all(|c| c.is_ascii_digit()) && s.parse::<i64>().is_ok()
}

/// Returns true for strings that look like floats (contain `.` or `e`/`E`)
/// and parse cleanly as f64. Rejects NaN and infinity strings.
fn is_float(s: &str) -> bool {
    let has_decimal_or_exp = s.contains('.') || s.contains('e') || s.contains('E');
    if !has_decimal_or_exp {
        return false;
    }
    match s.parse::<f64>() {
        Ok(v) => v.is_finite(),
        Err(_) => false,
    }
}

/// Returns true for the four boolean literals only, case-insensitive.
fn is_boolean(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no"
    )
}

/// Structural date check — no regex, no allocation beyond the split.
/// Accepts ISO-8601 dates (YYYY-MM-DD) and datetimes (YYYY-MM-DDTHH:MM:SS).
/// Does not validate calendar correctness — that is out of scope for schema inference.
fn is_date_like(s: &str) -> bool {
    // Minimum: YYYY-MM-DD = 10 chars
    if s.len() < 10 {
        return false;
    }

    let b = s.as_bytes();

    // Use get() throughout — clippy::indexing_slicing forbids direct indexing
    let date_part = match b.get(..10) {
        Some(d) => d,
        None => return false,
    };

    // YYYY-MM-DD structural check
    if !date_part.get(..4).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) {
        return false;
    }
    if date_part.get(4) != Some(&b'-') {
        return false;
    }
    if !date_part.get(5..7).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) {
        return false;
    }
    if date_part.get(7) != Some(&b'-') {
        return false;
    }
    if !date_part.get(8..10).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) {
        return false;
    }

    // If there's more, it must be a datetime: T or space then HH:MM:SS
    if s.len() > 10 {
        if s.len() < 19 {
            return false;
        }
        let sep = b.get(10);
        if sep != Some(&b'T') && sep != Some(&b' ') {
            return false;
        }
        // HH:MM:SS
        if !b.get(11..13).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }
        if b.get(13) != Some(&b':') { return false; }
        if !b.get(14..16).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }
        if b.get(16) != Some(&b':') { return false; }
        if !b.get(17..19).is_some_and(|v| v.iter().all(|c| c.is_ascii_digit())) { return false; }
    }

    true
}

// ── Public parse entry point ──────────────────────────────────────────────────

/// Parse a CSV string and return a [`SchemaOutput`].
///
/// # Errors
/// Returns [`SchemaError`] if any hard cap is exceeded or the input is
/// structurally invalid. Never returns raw cell content in any error variant.
pub fn parse_csv(input: &str) -> Result<SchemaOutput, SchemaError> {
    if input.trim().is_empty() {
        return Err(SchemaError::EmptyInput);
    }

    // The csv crate silently drops blank lines; replace each one with a
    // single empty quoted field so it arrives as an all-empty record.
    let patched: std::borrow::Cow<str> = if input.contains("\n\n") || input.contains("\r\n\r\n") {
        std::borrow::Cow::Owned(
            input
                .lines()
                .map(|line| if line.trim().is_empty() { "\"\"" } else { line })
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        std::borrow::Cow::Borrowed(input)
    };

    let mut reader = ReaderBuilder::new()
        .flexible(true)       // allow rows with different column counts
        .trim(csv::Trim::All) // trim whitespace from fields
        .from_reader(patched.as_bytes());

    // ── Header row ────────────────────────────────────────────────────────────
    let headers = reader
        .headers()
        .map_err(|e| {
            let pos = e.position();
            SchemaError::CsvParseFailed {
                row: pos.map_or(0, |p| p.line() as usize),
                column: None,
            }
        })?
        .clone();

    if headers.is_empty() {
        return Err(SchemaError::EmptyInput);
    }

    let col_count = headers.len();
    if col_count > MAX_COLS {
        return Err(SchemaError::TooManyColumns {
            limit: MAX_COLS,
            actual: col_count,
        });
    }

    // ── Initialise one accumulator per column ─────────────────────────────────
    let mut accumulators: Vec<ColumnAccumulator> = headers
        .iter()
        .enumerate()
        .map(|(i, name)| {
            // Sanitize: truncate to 256 chars, no HTML escaping (consumer's job)
            let sanitized: String = name.chars().take(256).collect();
            ColumnAccumulator::new(sanitized, i)
        })
        .collect::<Result<Vec<_>, _>>()?;

    // ── Data rows ─────────────────────────────────────────────────────────────
    let mut row_count: u64 = 0;
    let mut truncated = false;

    for result in reader.records() {
        if row_count as usize >= MAX_ROWS {
            truncated = true;
            break;
        }

        let record = result.map_err(|e| {
            let pos = e.position();
            SchemaError::CsvParseFailed {
                row: pos.map_or(row_count as usize, |p| p.line() as usize),
                column: None,
            }
        })?;

        // Guard against rows with more columns than the header declared
        let effective_cols = record.len().min(col_count);

        for col_idx in 0..effective_cols {
            let cell = record
                .get(col_idx)
                .unwrap_or(""); // flexible mode: missing trailing fields → empty

            let acc = accumulators
                .get_mut(col_idx)
                .ok_or(SchemaError::CsvParseFailed {
                    row: row_count as usize,
                    column: Some(col_idx),
                })?;

            acc.observe(cell, row_count as usize, col_idx)?;
        }

        // Any columns not present in this row (short rows) count as null
        for col_idx in effective_cols..col_count {
            let acc = accumulators
                .get_mut(col_idx)
                .ok_or(SchemaError::CsvParseFailed {
                    row: row_count as usize,
                    column: Some(col_idx),
                })?;
            acc.null_count += 1;
        }

        row_count += 1;
    }

    if row_count == 0 {
        return Err(SchemaError::EmptyInput);
    }

    // ── Finalise ──────────────────────────────────────────────────────────────
    let columns = accumulators
        .into_iter()
        .map(|acc| acc.finish(row_count))
        .collect();

    Ok(SchemaOutput {
        row_count,
        truncated,
        detected_format: "csv".to_string(),
        schemasniff_version: env!("CARGO_PKG_VERSION").to_string(),
        chunk_count: 1,
        columns,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── infer_cell_type ───────────────────────────────────────────────────────

    #[test]
    fn cell_integers() {
        assert_eq!(infer_cell_type("0"),          InferredType::Integer);
        assert_eq!(infer_cell_type("42"),         InferredType::Integer);
        assert_eq!(infer_cell_type("-17"),        InferredType::Integer);
        assert_eq!(infer_cell_type("9223372036854775807"), InferredType::Integer); // i64::MAX
    }

    #[test]
    fn cell_leading_zero_is_string() {
        // Leading zeros almost always mean IDs, zip codes, etc.
        assert_eq!(infer_cell_type("007"),  InferredType::String);
        assert_eq!(infer_cell_type("0042"), InferredType::String);
    }

    #[test]
    fn cell_floats() {
        assert_eq!(infer_cell_type("3.14"),  InferredType::Float);
        assert_eq!(infer_cell_type("-0.5"),  InferredType::Float);
        assert_eq!(infer_cell_type("1e10"),  InferredType::Float);
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
        assert_eq!(infer_cell_type("2024-01-15"),             InferredType::Date);
        assert_eq!(infer_cell_type("2024-01-15T09:30:00"),    InferredType::Date);
        assert_eq!(infer_cell_type("2024-01-15 09:30:00"),    InferredType::Date);
        assert_eq!(infer_cell_type("2024-1-1"),               InferredType::String); // not zero-padded
        assert_eq!(infer_cell_type("15-01-2024"),             InferredType::String); // wrong order
    }

    #[test]
    fn cell_empty_is_unknown() {
        assert_eq!(infer_cell_type(""),    InferredType::Unknown);
        assert_eq!(infer_cell_type("   "), InferredType::Unknown);
    }

    // ── parse_csv happy paths ─────────────────────────────────────────────────

    #[test]
    fn happy_path_10_rows() {
        let csv = "\
name,age,score,active,joined
Alice,30,9.5,true,2023-01-01
Bob,25,8.0,false,2023-02-14
Carol,40,7.2,true,2023-03-10
Dave,35,6.8,false,2023-04-05
Eve,28,9.9,true,2023-05-20
Frank,52,5.5,false,2023-06-30
Grace,19,8.8,true,2023-07-07
Heidi,44,7.1,false,2023-08-01
Ivan,31,6.0,true,2023-09-15
Judy,27,9.2,false,2023-10-31";

        let out = parse_csv(csv).expect("should parse");
        assert_eq!(out.row_count, 10);
        assert!(!out.truncated);
        assert_eq!(out.detected_format, "csv");
        assert_eq!(out.columns.len(), 5);

        let name_col = &out.columns[0];
        assert_eq!(name_col.name, "name");
        assert_eq!(name_col.inferred_type, InferredType::String);
        assert_eq!(name_col.null_count, 0);

        let age_col = &out.columns[1];
        assert_eq!(age_col.inferred_type, InferredType::Integer);
        assert_eq!(age_col.numeric_min, Some(19.0));
        assert_eq!(age_col.numeric_max, Some(52.0));

        let score_col = &out.columns[2];
        assert_eq!(score_col.inferred_type, InferredType::Float);

        let active_col = &out.columns[3];
        assert_eq!(active_col.inferred_type, InferredType::Boolean);

        let joined_col = &out.columns[4];
        assert_eq!(joined_col.inferred_type, InferredType::Date);
    }

    #[test]
    fn unicode_column_names() {
        let csv = "名前,年齢\nAlice,30\nBob,25";
        let out = parse_csv(csv).expect("unicode headers must parse");
        assert_eq!(out.columns[0].name, "名前");
        assert_eq!(out.columns[1].name, "年齢");
    }

    #[test]
    fn column_name_truncated_to_256_chars() {
        let long_name = "a".repeat(300);
        let csv = format!("{long_name},age\nAlice,30");
        let out = parse_csv(&csv).expect("should parse");
        assert_eq!(out.columns[0].name.chars().count(), 256);
    }

    #[test]
    fn empty_input_error() {
        assert!(matches!(parse_csv(""), Err(SchemaError::EmptyInput)));
        assert!(matches!(parse_csv("   "), Err(SchemaError::EmptyInput)));
    }

    #[test]
    fn header_only_is_empty_input() {
        // A CSV with headers but no data rows should be EmptyInput
        assert!(matches!(parse_csv("name,age\n"), Err(SchemaError::EmptyInput)));
    }

    #[test]
    fn too_many_columns_error() {
        // Build a header row with MAX_COLS + 1 columns
        let headers: Vec<String> = (0..=MAX_COLS).map(|i| format!("col{i}")).collect();
        let csv = headers.join(",");
        assert!(matches!(
            parse_csv(&csv),
            Err(SchemaError::TooManyColumns { limit: _, actual: _ })
        ));
    }

    #[test]
    fn oversized_cell_treated_as_null() {
        let big_cell = "x".repeat(MAX_CELL_BYTES + 1);
        let csv = format!("value\n{big_cell}\nnormal");
        let out = parse_csv(&csv).expect("should not error on oversized cell");
        // The oversized cell must be counted as null, not error
        assert_eq!(out.columns[0].null_count, 1);
        assert_eq!(out.row_count, 2);
    }

    #[test]
    fn null_ratio_correct() {
        let csv = "v\nfoo\n\nbar\n\nbaz";
        let out = parse_csv(csv).expect("parse");
        assert_eq!(out.columns[0].null_count, 2);
        assert!((out.columns[0].null_ratio - 0.4).abs() < 1e-9);
    }

    #[test]
    fn short_rows_padded_as_null() {
        // Row 2 has only 1 field; the second column should count it as null
        let csv = "a,b\nfoo,bar\nbaz";
        let out = parse_csv(csv).expect("parse");
        assert_eq!(out.columns[1].null_count, 1);
    }

    #[test]
    fn numeric_bounds_not_set_for_string_col() {
        let csv = "name\nAlice\nBob";
        let out = parse_csv(csv).expect("parse");
        assert_eq!(out.columns[0].numeric_min, None);
        assert_eq!(out.columns[0].numeric_max, None);
    }

    #[test]
    fn truncation_flag_set_at_max_rows() {
        // Build a CSV with MAX_ROWS + 2 data rows
        let mut csv = String::from("n\n");
        for i in 0..(MAX_ROWS + 2) {
            csv.push_str(&format!("{i}\n"));
        }
        let out = parse_csv(&csv).expect("parse");
        assert!(out.truncated);
        assert_eq!(out.row_count as usize, MAX_ROWS);
    }
}