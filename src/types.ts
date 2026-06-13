/**
 * Inferred column type — mirrors the Rust `InferredType` enum exactly.
 * Variants are snake_case to match the serde serialization.
 */
export type InferredType =
  | "integer"
  | "float"
  | "boolean"
  | "date"
  | "string"
  | "unknown";

/**
 * Metadata for a single column — mirrors the Rust `ColumnMeta` struct.
 * Never contains raw cell values.
 */
export interface ColumnMeta {
  /** Sanitized column name, truncated to 256 chars. Not HTML-escaped. */
  name: string;

  /** Zero-based column position in the source. */
  index: number;

  /** Inferred dominant type for this column. */
  inferred_type: InferredType;

  /** True if any null/empty/missing values were found. */
  nullable: boolean;

  /** Count of null/empty/missing cells. */
  null_count: number;

  /** Ratio of nulls to total rows — always in [0.0, 1.0]. */
  null_ratio: number;

  /**
   * Minimum numeric value seen.
   * Only present when inferred_type is "integer" or "float".
   * Always finite — never NaN or Infinity.
   */
  numeric_min: number | null;

  /**
   * Maximum numeric value seen.
   * Only present when inferred_type is "integer" or "float".
   * Always finite — never NaN or Infinity.
   */
  numeric_max: number | null;

  /**
   * Approximate distinct-value count via HyperLogLog (±2% per chunk).
   * When chunk_count > 1 on the parent SchemaResult, this value is the
   * sum of per-chunk estimates — it over-counts repeated values across
   * chunk boundaries. Treat it as an upper bound, not an exact estimate.
   */
  cardinality_estimate: number;
}

/**
 * Complete schema inference result — mirrors the Rust `SchemaOutput` struct.
 * Never contains raw cell data.
 */
export interface SchemaResult {
  /** Total data rows processed (header excluded for CSV). */
  row_count: number;

  /**
   * True if input exceeded the 1,000,000 row limit.
   * Schema reflects only the rows that were processed.
   */
  truncated: boolean;

  /** Detected input format. */
  detected_format: "csv" | "json" | "ndjson";

  /** Library version that produced this output. */
  schemasniff_version: string;

  /**
   * Number of chunks the input was split into for processing.
   * 1 means the file was small enough to process in a single pass.
   * Greater than 1 means the file was automatically chunked internally.
   * Useful for understanding cardinality estimate accuracy —
   * estimates are less precise when chunk_count > 1.
   */
  chunk_count: number;

  /** Column metadata in source order. */
  columns: ColumnMeta[];
}

// ── Options ───────────────────────────────────────────────────────────────────

/**
 * Optional configuration for inferSchema.
 */
export interface InferSchemaOptions {
  /**
   * Called during processing of large files with a progress value in [0, 1].
   * Not called for small files (under 10 MB) since they complete instantly.
   * Use this to drive a progress bar in your UI.
   *
   * @example
   * const result = await inferSchema(text, {
   *     onProgress: (p) => progressBar.style.width = `${p * 100}%`
   * });
   */
  onProgress?: (fraction: number) => void;
}

// ── Error types ───────────────────────────────────────────────────────────────

/**
 * Discriminated union of all structured errors schemasniff can return.
 * The `error` field is the discriminant — use it in a switch/if chain.
 * No variant ever contains raw input data.
 *
 * @example
 * const result = inferSchema(input);
 * if ("error" in result) {
 *   switch (result.error) {
 *     case "input_too_large":
 *       console.error(`Too large: ${result.actual_bytes} bytes`);
 *       break;
 *     case "too_many_columns":
 *       console.error(`${result.actual} columns exceeds limit of ${result.limit}`);
 *       break;
 *   }
 * }
 */
export type SchemaError =
  | {
      /** Input exceeded the JS-side 10 MB byte cap. */
      error: "input_too_large";
      limit_bytes: number;
      actual_bytes: number;
    }
  | {
      /** Column count exceeded MAX_COLS (1,024). */
      error: "too_many_columns";
      limit: number;
      actual: number;
    }
  | {
      /** Row count exceeded MAX_ROWS (1,000,000). Parsing stopped. */
      error: "row_limit_reached";
      limit: number;
    }
  | {
      /** JSON nesting depth exceeded MAX_JSON_DEPTH (32). */
      error: "nesting_too_deep";
      limit: number;
      detected_at_row: number;
    }
  | {
      /** Invalid UTF-8 encoding or NUL byte detected. */
      error: "encoding_error";
      /** Byte offset of the first invalid byte, if known. */
      byte_offset: number | null;
    }
  | {
      /** CSV structural parse failure. Position only — no cell content. */
      error: "csv_parse_failed";
      row: number;
      column: number | null;
    }
  | {
      /** JSON structural parse failure. */
      error: "json_parse_failed";
      byte_offset: number | null;
    }
  | {
      /** Input was empty or whitespace-only. */
      error: "empty_input";
    }
  | {
      /** Format not recognised as CSV, JSON, or NDJSON. */
      error: "unrecognized_format";
    }
  | {
      /** JS-side type guard failed — input was not a string. */
      error: "invalid_input";
      message: string;
    };

// ── Discriminant literal type ─────────────────────────────────────────────────

/** All possible error discriminant strings — useful for exhaustive switches. */
export type SchemaErrorKind = SchemaError["error"];

// ── Type guards ───────────────────────────────────────────────────────────────

/**
 * Returns true if the value is a SchemaError.
 * Use this to distinguish success from failure after calling inferSchema.
 *
 * @example
 * const result = inferSchema(input);
 * if (isSchemaError(result)) {
 *   // result is SchemaError
 * } else {
 *   // result is SchemaResult
 * }
 */
export function isSchemaError(
  value: SchemaResult | SchemaError
): value is SchemaError {
  return "error" in value;
}

/**
 * Returns true if the value is a SchemaResult.
 */
export function isSchemaResult(
  value: SchemaResult | SchemaError
): value is SchemaResult {
  return !("error" in value);
}

// ── Return type alias ─────────────────────────────────────────────────────────

/**
 * The return type of `inferSchema` — either a result or a structured error.
 * Check with `isSchemaError` / `isSchemaResult` before accessing fields.
 */
export type InferSchemaReturn = SchemaResult | SchemaError;