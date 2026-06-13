#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|depth: u8| {
    // Build JSON nested to `depth` levels — tests the depth guard at all values
    let mut json = String::from("[{");
    for _ in 0..depth {
        json.push_str("\"k\":{");
    }
    json.push_str("\"v\":1");
    for _ in 0..depth {
        json.push('}');
    }
    json.push_str("}]");

    let _ = schemasniff::json_parser::parse_json(&json);
});