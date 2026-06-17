# Changelog

All notable changes to schemasniff are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-06-XX

### Added
- Initial release of schemasniff — zero-backend schema inference for CSV and JSON, running entirely in the browser via WebAssembly
- CSV parsing with RFC 4180 tokenization via the `csv` crate
- JSON parsing supporting object-array (`[{},{}]`) and NDJSON (`{}\n{}\n`) formats
- Deterministic type inference: integer → float → boolean → date → string priority ladder, no regex
- Per-column statistics: null count, null ratio, numeric min/max, distinct-value cardinality
- Inline HyperLogLog++ cardinality estimator (±2% error) — no external dependency, raw values never stored
- Five-layer security architecture: JS-side byte cap, input type guard, row/column/cell hard caps, pure-function structural guarantee, output sanitization
- Automatic internal chunking for files over 10 MB — consumers call one function regardless of file size
- Closed `SchemaError` enum with position-only error variants — compile-time size seal prevents future leakage of raw cell content
- Full TypeScript types with JSDoc on every field — `SchemaResult`, `SchemaError`, `ColumnMeta`, `InferredType`
- `cargo audit` and `cargo deny` integrated into CI, running on every push and daily via scheduled workflow
- Fuzz testing via `cargo-fuzz` against random bytes, oversized inputs, deeply nested JSON, NUL bytes, and non-UTF-8 sequences
- `#![forbid(unsafe_code)]` and `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]` enforced at the crate root
- WASM binary SRI hash published with every GitHub release for integrity verification
- All GitHub Actions pinned to commit SHAs for supply chain hardening

### Security
- No known vulnerabilities at time of release
- Dependency tree minimised: HyperLogLog implemented inline rather than via external crate, reducing total dependency count from 53 to ~40

[0.1.0]: https://github.com/shashik116/schemasniff/releases/tag/v0.1.0
