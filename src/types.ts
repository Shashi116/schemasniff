/** Inferred column type — mirrors the Rust `InferredType` enum. */
export type InferredType =
  | "integer"
  | "float"
  | "boolean"
  | "date"
  | "string"
  | "unknown";

/** Per-column metadata. Never contains raw cell values. */
export interface ColumnMeta {
  /** Truncated to 256 chars. */
  name: string;
  index: number;
  inferred_type: InferredType;
  nullable: boolean;
  null_count: number;
  /** Always in [0.0, 1.0]. */
  null_ratio: number;
  /** Only set for integer/float columns. Always finite. */
  numeric_min: number | null;
  /** Only set for integer/float columns. Always finite. */
  numeric_max: number | null;
  /** HyperLogLog estimate (±2%). Upper bound when chunk_count > 1. */
  cardinality_estimate: number;
}

/** Schema inference result. Never contains raw cell data. */
export interface SchemaResult {
  row_count: number;
  truncated: boolean;
  detected_format: "csv" | "json" | "ndjson";
  schemasniff_version: string;
  /** > 1 means cardinality estimates are less precise. */
  chunk_count: number;
  columns: ColumnMeta[];
}

export interface InferSchemaOptions {
  /**
   * Called with progress in [0, 1] while processing large files (> 10 MB).
   * @example
   * await inferSchema(text, { onProgress: (p) => bar.style.width = `${p * 100}%` });
   */
  onProgress?: (fraction: number) => void;
}

/**
 * Discriminated union of all errors schemasniff can return.
 * Use the `error` field as the discriminant.
 */
export type SchemaError =
  | { error: "input_too_large";   limit_bytes: number; actual_bytes: number }
  | { error: "too_many_columns";  limit: number; actual: number }
  | { error: "row_limit_reached"; limit: number }
  | { error: "nesting_too_deep";  limit: number; detected_at_row: number }
  | { error: "encoding_error";    byte_offset: number | null }
  | { error: "csv_parse_failed";  row: number; column: number | null }
  | { error: "json_parse_failed"; byte_offset: number | null }
  | { error: "empty_input" }
  | { error: "unrecognized_format" }
  | { error: "invalid_input"; message: string };

/** All possible error discriminant strings. */
export type SchemaErrorKind = SchemaError["error"];

/** Returns true if `value` is a SchemaError. */
export function isSchemaError(
  value: SchemaResult | SchemaError
): value is SchemaError {
  return "error" in value;
}

/** Returns true if `value` is a SchemaResult. */
export function isSchemaResult(
  value: SchemaResult | SchemaError
): value is SchemaResult {
  return !("error" in value);
}

/** Return type of `inferSchema`. */
export type InferSchemaReturn = SchemaResult | SchemaError;
