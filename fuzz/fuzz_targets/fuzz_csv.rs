#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Attempt to interpret arbitrary bytes as UTF-8 and parse as CSV.
    // Must never panic — only Ok or Err are acceptable outcomes.
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = schemasniff::csv_parser::parse_csv(s);
    }
});