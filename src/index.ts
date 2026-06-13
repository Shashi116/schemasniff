import type {
  InferSchemaReturn,
  InferSchemaOptions,
  SchemaResult,
  SchemaError,
  ColumnMeta,
} from "./types";
import { isSchemaError } from "./types";

export type {
  InferSchemaReturn,
  InferSchemaOptions,
  SchemaResult,
  SchemaError,
  SchemaErrorKind,
  ColumnMeta,
  InferredType,
} from "./types";
export { isSchemaError, isSchemaResult } from "./types";

// ── Private constants ─────────────────────────────────────────────────────────
const CHUNK_THRESHOLD = 10 * 1024 * 1024; // files over this are split into chunks
const CHUNK_SIZE      =  4 * 1024 * 1024; // max bytes per chunk

// ── Lazy WASM initialisation ──────────────────────────────────────────────────

type WasmModule = { infer_schema: (input: string) => unknown };
let wasmModule: WasmModule | null = null;
let wasmReady: Promise<void> | null = null;

function ensureWasm(): Promise<void> {
  if (wasmReady === null) {
    wasmReady = import("../pkg/schemasniff.js").then((m: WasmModule) => {
      wasmModule = m;
    }).catch((err) => {
      wasmReady = null; // allow retry on next call
      throw err;
    });
  }
  return wasmReady;
}

// ── Call-once lock ────────────────────────────────────────────────────────────
let isRunning = false;

// ── Raw WASM call ─────────────────────────────────────────────────────────────

function callWasm(input: string): InferSchemaReturn {
  try {
    const raw = wasmModule!.infer_schema(input) as Record<string, unknown>;
    if (typeof raw === "object" && raw !== null && "error" in raw) {
      return raw as unknown as SchemaError;
    }
    return raw as unknown as SchemaResult;
  } catch {
    return { error: "unrecognized_format" };
  }
}

// ── Chunk splitter ────────────────────────────────────────────────────────────
// O(n) — each line is encoded exactly once. Byte length is tracked as a running
// counter; the chunk string is only built (parts.join) when flushing a full chunk.

function splitIntoChunks(input: string): string[] {
  // .length ≈ byte count for ASCII — avoids encoding the whole string unless near boundary
  const fastLen = input.length;
  const byteLen = fastLen > CHUNK_THRESHOLD * 0.8
    ? new TextEncoder().encode(input).length
    : fastLen;

  if (byteLen <= CHUNK_THRESHOLD) return [input];

  const encoder    = new TextEncoder();
  const lines      = input.split("\n");
  const header     = lines[0] ?? "";
  const headerBytes = encoder.encode(header).length;
  const chunks: string[] = [];
  const parts: string[]  = [header];
  let currentBytes = headerBytes;

  for (let i = 1; i < lines.length; i++) {
    const line      = lines[i] ?? "";
    const lineBytes = encoder.encode(line).length + 1; // +1 for "\n"

    if (currentBytes + lineBytes > CHUNK_SIZE && parts.length > 1) {
      chunks.push(parts.join("\n"));
      parts.length = 0;
      parts.push(header, line);
      currentBytes = headerBytes + 1 + lineBytes;
    } else {
      parts.push(line);
      currentBytes += lineBytes;
    }
  }

  if (parts.length > 1) chunks.push(parts.join("\n"));
  return chunks.length > 0 ? chunks : [input];
}

// ── Result merger ─────────────────────────────────────────────────────────────
// Combines N per-chunk SchemaResults into one. For each column:
//   - null_count and cardinality_estimate are summed
//   - numeric_min/max are taken as global min/max across chunks
//   - inferred_type: first non-unknown type wins
//   - null_ratio is recomputed from merged totals

function mergeResults(results: SchemaResult[], chunkCount: number): SchemaResult {
  const base = results[0]!;

  const colMap = new Map<string, {
    null_count: number; numeric_min: number | null; numeric_max: number | null;
    cardinality_estimate: number; inferred_type: ColumnMeta["inferred_type"];
    nullable: boolean; index: number;
  }>();

  for (const col of base.columns) {
    colMap.set(col.name, {
      null_count: col.null_count, numeric_min: col.numeric_min ?? null,
      numeric_max: col.numeric_max ?? null, cardinality_estimate: col.cardinality_estimate,
      inferred_type: col.inferred_type, nullable: col.nullable, index: col.index,
    });
  }

  let total_row_count = base.row_count;
  let any_truncated   = base.truncated;

  for (let i = 1; i < results.length; i++) {
    const r = results[i]!;
    total_row_count += r.row_count;
    if (r.truncated) any_truncated = true;

    for (const col of r.columns) {
      const acc = colMap.get(col.name);
      if (!acc) {
        colMap.set(col.name, {
          null_count: col.null_count, numeric_min: col.numeric_min ?? null,
          numeric_max: col.numeric_max ?? null, cardinality_estimate: col.cardinality_estimate,
          inferred_type: col.inferred_type, nullable: col.nullable, index: col.index,
        });
      } else {
        acc.null_count           += col.null_count;
        acc.cardinality_estimate += col.cardinality_estimate;
        acc.nullable              = acc.nullable || col.nullable;
        const mn = col.numeric_min ?? null;
        const mx = col.numeric_max ?? null;
        if (mn !== null) acc.numeric_min = acc.numeric_min === null ? mn : Math.min(acc.numeric_min, mn);
        if (mx !== null) acc.numeric_max = acc.numeric_max === null ? mx : Math.max(acc.numeric_max, mx);
        if (acc.inferred_type === "unknown" && col.inferred_type !== "unknown")
          acc.inferred_type = col.inferred_type;
      }
    }
  }

  const columns: ColumnMeta[] = Array.from(colMap.entries())
    .sort((a, b) => a[1].index - b[1].index)
    .map(([name, acc]) => ({
      name, index: acc.index, inferred_type: acc.inferred_type,
      nullable: acc.nullable, null_count: acc.null_count,
      null_ratio: total_row_count > 0 ? acc.null_count / Number(total_row_count) : 0,
      numeric_min: acc.numeric_min, numeric_max: acc.numeric_max,
      cardinality_estimate: acc.cardinality_estimate,
    }));

  return {
    row_count: total_row_count, truncated: any_truncated,
    detected_format: base.detected_format,
    schemasniff_version: base.schemasniff_version,
    chunk_count: chunkCount, columns,
  };
}

// ── Chunked path ──────────────────────────────────────────────────────────────
// setTimeout(0) between chunks yields to the event loop so the browser can
// repaint the progress bar before the next synchronous WASM call.

async function runChunked(
  chunks: string[],
  onProgress?: (fraction: number) => void,
): Promise<InferSchemaReturn> {
  const results: SchemaResult[] = [];

  for (let i = 0; i < chunks.length; i++) {
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    const result = callWasm(chunks[i]!);
    if (isSchemaError(result)) return result;
    results.push(result);
    onProgress?.((i + 1) / chunks.length);
  }

  if (results.length === 0) return { error: "empty_input" };
  if (results.length === 1) return { ...results[0]!, chunk_count: 1 };
  return mergeResults(results, chunks.length);
}

// ── Public API ────────────────────────────────────────────────────────────────

export async function inferSchema(
  input: unknown,
  options?: InferSchemaOptions,
): Promise<InferSchemaReturn> {
  if (typeof input !== "string") {
    return { error: "invalid_input", message: "input must be a string" };
  }

  if (isRunning) {
    return { error: "invalid_input", message: "concurrent calls are not supported" };
  }

  isRunning = true;
  try {
    await ensureWasm();

    const chunks = splitIntoChunks(input);

    if (chunks.length === 1) {
      const result = callWasm(chunks[0]!);
      if (isSchemaError(result)) return result;
      return { ...result, chunk_count: 1 };
    }

    return await runChunked(chunks, options?.onProgress);
  } catch {
    return { error: "unrecognized_format" };
  } finally {
    isRunning = false;
  }
}
