#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Pass raw bytes — some will be valid UTF-8, some won't.
    // The security layer must handle both without panicking.
    match std::str::from_utf8(data) {
        Ok(s) => {
            // Valid UTF-8 — may contain NUL bytes, which validate_input rejects
            let _ = schemasniff::security::validate_input(s);
            let _ = schemasniff::csv_parser::parse_csv(s);
            let _ = schemasniff::json_parser::parse_json(s);
        }
        Err(_) => {
            // Invalid UTF-8 — simulate what the WASM boundary does:
            // lossy-convert and confirm the error path is clean
            let lossy = String::from_utf8_lossy(data);
            let _ = schemasniff::security::validate_input(&lossy);
        }
    }
});