# schemasniff

Fast, secure, zero-backend schema inference for CSV and JSON — runs entirely in the browser via WebAssembly. No server, no upload, no data leaves the user's machine.

[![npm version](https://img.shields.io/npm/v/schemasniff.svg)](https://www.npmjs.com/package/schemasniff)
[![CI](https://github.com/Shashi116/schemasniff/actions/workflows/ci.yml/badge.svg)](https://github.com/Shashi116/schemasniff/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

## Why schemasniff

Drop a CSV or JSON file into a browser tab and get back a typed schema —
column names, inferred types, null counts, numeric ranges, and cardinality
estimates — without ever sending the file to a server. Built in Rust,
compiled to WASM, with a five-layer security model designed for untrusted
user-uploaded files.

- **Zero backend** — runs entirely client-side; sensitive data never leaves the browser
- **Fast** — Rust + WASM; a 100k-row, 12.5 MB CSV processes in 1.5–3 seconds
- **Safe by construction** — `#![forbid(unsafe_code)]`, no `unwrap()` in production paths, fuzzed against malformed input
- **Handles large files automatically** — internal chunking kicks in above 10 MB with no API change
- **Tiny** — 208 KB WASM binary, ~91 KB gzipped npm package

## Install

```bash
npm install schemasniff
```

## Quick start

```typescript
import { inferSchema, isSchemaError } from "schemasniff";

const csv = `name,age,score
Alice,30,9.5
Bob,25,8.0`;

const result = await inferSchema(csv);

if (isSchemaError(result)) {
  console.error(result.error); // e.g. "empty_input"
} else {
  console.log(result.row_count);        // 2
  console.log(result.detected_format);  // "csv"
  console.log(result.columns);
  // [
  //   { name: "name",  inferred_type: "string",  null_count: 0, ... },
  //   { name: "age",   inferred_type: "integer", numeric_min: 25, numeric_max: 30, ... },
  //   { name: "score", inferred_type: "float",   numeric_min: 8.0, numeric_max: 9.5, ... }
  // ]
}
```

### Large files — no code change needed

```typescript
// Files over 10 MB are automatically chunked internally.
// Same function, same return type — just pass an optional progress callback.
const result = await inferSchema(largeCsvText, {
  onProgress: (fraction) => updateProgressBar(fraction * 100)
});

if (!isSchemaError(result) && result.chunk_count > 1) {
  console.log(`Processed in ${result.chunk_count} chunks`);
  // cardinality_estimate is an upper bound when chunk_count > 1
}
```

## API Reference

### `inferSchema(input, options?)`

```typescript
function inferSchema(
  input: unknown,
  options?: InferSchemaOptions
): Promise<SchemaResult | SchemaError>
```

| Parameter | Type | Description |
|---|---|---|
| `input` | `unknown` | The file content as a string. Non-string input returns `invalid_input`. |
| `options.onProgress` | `(fraction: number) => void` | Called during chunked processing of files over 10 MB. Not called for smaller files. |

Returns a `Promise` resolving to either a `SchemaResult` (success) or a `SchemaError` (failure). Use `isSchemaError()` / `isSchemaResult()` to discriminate.

### `SchemaResult`

| Field | Type | Description |
|---|---|---|
| `row_count` | `number` | Total data rows processed (excludes header for CSV) |
| `truncated` | `boolean` | `true` if input exceeded 1,000,000 rows |
| `detected_format` | `"csv" \| "json" \| "ndjson"` | Auto-detected input format |
| `schemasniff_version` | `string` | Library version that produced this output |
| `chunk_count` | `number` | `1` for single-pass; `>1` if internally chunked |
| `columns` | `ColumnMeta[]` | Per-column metadata, in source order |

### `ColumnMeta`

| Field | Type | Description |
|---|---|---|
| `name` | `string` | Column name, truncated to 256 chars. **Not HTML-escaped** — escape it yourself before rendering. |
| `index` | `number` | Zero-based column position |
| `inferred_type` | `"integer" \| "float" \| "boolean" \| "date" \| "string" \| "unknown"` | Dominant type by majority vote |
| `nullable` | `boolean` | `true` if any null/empty values found |
| `null_count` | `number` | Count of null/empty cells |
| `null_ratio` | `number` | `null_count / row_count`, range `[0, 1]` |
| `numeric_min` | `number \| null` | Only set for `integer`/`float` columns |
| `numeric_max` | `number \| null` | Only set for `integer`/`float` columns |
| `cardinality_estimate` | `number` | Approximate distinct values (±2% error). Upper bound if `chunk_count > 1`. |

### `SchemaError`

A discriminated union on the `error` field:

| `error` value | Extra fields | Meaning |
|---|---|---|
| `input_too_large` | `limit_bytes`, `actual_bytes` | Input exceeded internal byte cap |
| `too_many_columns` | `limit`, `actual` | Header has more than 1,024 columns |
| `row_limit_reached` | `limit` | Input has more than 1,000,000 rows (use `truncated` flag instead — this variant is rare) |
| `nesting_too_deep` | `limit`, `detected_at_row` | JSON nested more than 32 levels |
| `encoding_error` | `byte_offset` | Invalid UTF-8 or NUL byte detected |
| `csv_parse_failed` | `row`, `column` | CSV structurally malformed at this position |
| `json_parse_failed` | `byte_offset` | JSON/NDJSON structurally malformed at this position |
| `empty_input` | — | Input was empty, whitespace, or header-only with no data rows |
| `unrecognized_format` | — | Input didn't match CSV, JSON, or NDJSON structure |
| `invalid_input` | `message` | Input was not a string |

**No `SchemaError` variant ever contains raw cell content** — only counts and positions. This is enforced by a compile-time check in the Rust source.

### Type guards

```typescript
import { isSchemaError, isSchemaResult } from "schemasniff";

const result = await inferSchema(input);

if (isSchemaError(result)) {
  // result is SchemaError — access result.error, result.limit_bytes, etc.
}

if (isSchemaResult(result)) {
  // result is SchemaResult — access result.row_count, result.columns, etc.
}
```

## Security model

schemasniff is designed to safely process **untrusted, user-uploaded files** in the browser. Five layers of defence:

1. **JS-side byte cap** — rejects oversized input before it reaches WASM
2. **Input type guard** — non-string input is rejected immediately
3. **Hard parsing caps** — `MAX_ROWS`, `MAX_COLS`, `MAX_CELL_BYTES`, `MAX_JSON_DEPTH` are compiled constants, not runtime config
4. **Pure function guarantee** — no I/O, no DOM access, no global mutable state; `#![forbid(unsafe_code)]`
5. **Output sanitization** — every result is validated post-parse to confirm no raw cell data leaked into any field

See [SECURITY.md](./SECURITY.md) for the full threat model and responsible disclosure policy.

## Limits

| Limit | Value | Behaviour when exceeded |
|---|---|---|
| Max rows | 1,000,000 | `truncated: true`, schema reflects rows processed so far |
| Max columns | 1,024 | `TooManyColumns` error, no rows processed |
| Max cell size | 65,536 bytes | Cell treated as `null`, parsing continues |
| Max JSON nesting | 32 levels | `NestingTooDeep` error |
| Internal chunk threshold | 10 MB | Automatic internal chunking, transparent to caller |
| Max total input size | 256 MB | `InputTooLarge` error |

## Verifying the WASM binary (SRI)

Every GitHub release publishes a SHA-256 SRI hash for `schemasniff_bg.wasm`. Verify the binary you received matches:

```bash
openssl dgst -sha256 -binary node_modules/schemasniff/schemasniff_bg.wasm | openssl base64 -A
```

Compare the output against the `SRI Hash` listed in the [release notes](https://github.com/Shashi116/schemasniff/releases) for the version you installed.

If loading via CDN with a `<script>` tag, use the `integrity` attribute directly:

```html
<script
  src="https://cdn.jsdelivr.net/npm/schemasniff@0.1.0/schemasniff.js"
  integrity="sha256-<hash-from-release-notes>"
  crossorigin="anonymous">
</script>
```

## Benchmarks

| Input | Rows | Size | Time (Chrome, M-series Mac) |
|---|---|---|---|
| Small CSV | 10 | < 1 KB | < 5 ms |
| Sales records | 100,000 | 12.5 MB | ~1.5–3 s (4 chunks) |

*Benchmarks recorded with `console.time` / `console.timeEnd` in the demo — see `demo/main.ts`.*

## Browser support

Tested in Chrome and Firefox. Any browser with WebAssembly support (all modern browsers) should work.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Hard rules: no `unsafe` code, no `unwrap()`/`expect()` in production paths, no new runtime dependency without discussion.

## License

Dual-licensed under [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE), at your option.
