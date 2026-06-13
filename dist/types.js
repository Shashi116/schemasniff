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
export function isSchemaError(value) {
    return "error" in value;
}
/**
 * Returns true if the value is a SchemaResult.
 */
export function isSchemaResult(value) {
    return !("error" in value);
}
//# sourceMappingURL=types.js.map