#![no_main]
use libfuzzer_sys::fuzz_target;
use schemasniff::{MAX_COLS, MAX_CELL_BYTES};

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Prepend a header that's exactly at the column cap
        let headers: String = (0..MAX_COLS)
            .map(|i| format!("col{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let combined = format!("{headers}\n{s}");
        let _ = schemasniff::csv_parser::parse_csv(&combined);

        // Wrap in a cell that's exactly at MAX_CELL_BYTES
        // Build padding manually — never use s as a format string
        let mut padded = String::from("v\n");
        padded.push_str(s);
        let pad_len = MAX_CELL_BYTES.saturating_sub(s.len());
        padded.extend(std::iter::repeat('0').take(pad_len));
        let _ = schemasniff::csv_parser::parse_csv(&padded);
    }
});