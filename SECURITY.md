# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅ Active  |

Only the latest minor version receives security fixes.
Patch releases are issued for confirmed vulnerabilities within 7 days of disclosure.

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Report vulnerabilities privately via GitHub's built-in security advisory system:

1. Go to the **Security** tab of this repository
2. Click **Report a vulnerability**
3. Fill in the details — include steps to reproduce, impact assessment, and any suggested fix

You will receive an acknowledgement within **48 hours** and a full response within **7 days**.
If you do not hear back, email the address listed on the npm package page.

### What to include in your report

- A description of the vulnerability and its potential impact
- Steps to reproduce (a minimal CSV or JSON input that triggers the issue)
- The version of schemasniff affected
- Whether you have a suggested fix or patch

### What to expect

- Acknowledgement within 48 hours
- A fix or mitigation plan within 7 days for critical issues
- Credit in the release notes if you wish (opt-in)
- No legal action for good-faith security research

---

## Threat Model

schemasniff is a **zero-backend schema inference library** that runs entirely
in the browser via WebAssembly. It accepts untrusted user-supplied CSV and JSON
text and returns a structured schema description. It never sends data to a server,
never writes to disk, and never accesses the DOM or network.

### What schemasniff protects against

#### 1. Memory exhaustion (OOM/DoS)
Untrusted input could attempt to exhaust browser memory via extremely large
files, wide CSVs, or deeply nested JSON.

**Mitigations:**
- Hard row cap: 1,000,000 rows — excess rows set `truncated: true`, parsing stops cleanly
- Hard column cap: 1,024 columns — excess columns return `TooManyColumns` immediately
- Hard cell cap: 65,536 bytes per cell — oversized cells are treated as null, never stored
- Hard nesting cap: 32 JSON levels — deeper structures return `NestingTooDeep` immediately
- JS-side byte cap: 10 MB checked before WASM call; Rust-side: 10 MB belt-and-suspenders

#### 2. Data leakage via error messages
A buggy library could accidentally echo cell contents back through error messages,
leaking sensitive data (PII, credentials, financial data) to the caller.

**Mitigations:**
- `SchemaError` is a closed enum — all fields are `usize` or `Option<usize>` only
- Compile-time size seal: a test asserts `size_of::<SchemaError>() <= 4 * size_of::<usize>()`
  — adding a `String` field causes a compile error, not a runtime bug
- `Display` impl is audited to contain only ASCII digits and static strings
- Output sanitization layer (`security::sanitize_output`) runs after parsing and before
  the result crosses the WASM boundary — validates no raw values entered any output field

#### 3. Panic-based crashes
A panic in WASM aborts the entire JS runtime. Malicious input could trigger panics
via integer overflow, index out of bounds, or unwrap on None.

**Mitigations:**
- `#![forbid(unsafe_code)]` — no raw pointers or unsafe blocks anywhere in the codebase
- `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]`
  — these lint errors block compilation if any production panic path is introduced
- All fallible operations use `.unwrap_or(fallback)` or `?` propagation
- Fuzz targets run in CI against random bytes, oversized inputs, deep nesting,
  NUL bytes, and non-UTF-8 sequences

#### 4. Supply chain attacks
A compromised dependency could inject malicious behaviour into the published package.

**Mitigations:**
- Dependency count reduced from 53 to ~40 crates — HyperLogLog implemented inline,
  eliminating the `hyperloglogplus` crate and its transitive deps
- `cargo deny check` enforces: no yanked crates, no unmaintained crates, licence allow-list
- `cargo audit` runs daily in CI via scheduled workflow — catches new advisories
  without requiring a code push
- WASM binary SRI hash published with every release — consumers can verify integrity

#### 5. Prototype pollution and JS-side injection
A malicious input could attempt to inject keys like `__proto__` or `constructor`
into the output object to pollute the JS prototype chain.

**Mitigations:**
- Output is serialized via `serde_wasm_bindgen::to_value` which produces a plain
  `JsValue` — not constructed via `eval` or string interpolation
- Column names are truncated to 256 chars and passed through as data fields inside
  a typed struct, never used as dynamic property keys
- `"sideEffects": false` in `package.json` — tree-shaking eliminates unused code paths

### What schemasniff does NOT protect against

- **The caller storing or transmitting inferred schema** — schemasniff returns a schema
  description; what the caller does with it is outside this library's scope
- **Correct calendar validation** — dates are detected structurally (YYYY-MM-DD pattern)
  not validated (2024-99-99 would be typed as `date`)
- **Exact cardinality for chunked files** — when `chunk_count > 1`, cardinality estimates
  are summed across chunks and may over-count values that appear in multiple chunks

### Dependency security

| Dependency | Purpose | Audit status |
|---|---|---|
| `wasm-bindgen` | WASM↔JS bridge | Maintained by the Rust/WASM WG |
| `serde` + `serde_json` | Serialization | Widely audited, no known CVEs |
| `csv` | RFC 4180 tokenization | Maintained, no known CVEs |
| `serde-wasm-bindgen` | Serde→JsValue | Actively maintained |

All dependencies are checked daily by `cargo audit` and `cargo deny`.
The full dependency tree is visible in `Cargo.lock`.

---

## Security Changelog

| Version | Issue | Fix |
|---------|-------|-----|
| 0.1.0   | Initial release — no known vulnerabilities | — |
