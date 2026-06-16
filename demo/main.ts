import { inferSchema, isSchemaError, isSchemaResult } from "schemasniff";
import type { SchemaResult, SchemaError, ColumnMeta } from "schemasniff";

// ── DOM refs ──────────────────────────────────────────────────────────────────

const dropZone      = document.getElementById("drop-zone")         as HTMLDivElement;
const fileInput     = document.getElementById("file-input")        as HTMLInputElement;
const loading       = document.getElementById("loading")           as HTMLDivElement;
const progressBar   = document.getElementById("progress-bar")      as HTMLDivElement;
const progressLabel = document.getElementById("progress-label")    as HTMLDivElement;
const truncBanner   = document.getElementById("truncation-banner") as HTMLDivElement;
const errorDiv      = document.getElementById("error")             as HTMLDivElement;
const outputDiv     = document.getElementById("output")            as HTMLDivElement;

// ── File reading ──────────────────────────────────────────────────────────────

async function readFileAsText(file: File): Promise<string> {
  const buffer = await file.arrayBuffer();
  return new TextDecoder("utf-8", { fatal: true }).decode(buffer);
}

// ── UI helpers ────────────────────────────────────────────────────────────────

function showLoading(): void {
  progressBar.style.width = "0%";
  progressLabel.textContent = "⏳ Analysing schema…";
  loading.classList.add("visible");
  truncBanner.classList.remove("visible");
  errorDiv.classList.remove("visible");
  errorDiv.textContent = "";
  outputDiv.innerHTML = "";
}

function hideLoading(): void {
  loading.classList.remove("visible");
  progressBar.style.width = "0%";
}

function setProgress(fraction: number): void {
  const pct = Math.round(fraction * 100);
  progressBar.style.width = `${pct}%`;
  progressLabel.textContent = `⏳ Analysing… ${pct}%`;
}

function renderError(err: SchemaError): void {
  const lines: string[] = [`error: ${err.error}`];
  if (err.error === "input_too_large") {
    lines.push(`limit:  ${(err.limit_bytes  / 1024 / 1024).toFixed(1)} MB`);
    lines.push(`actual: ${(err.actual_bytes / 1024 / 1024).toFixed(2)} MB`);
  } else if (err.error === "too_many_columns") {
    lines.push(`limit: ${err.limit}`, `actual: ${err.actual}`);
  } else if (err.error === "row_limit_reached") {
    lines.push(`limit: ${err.limit.toLocaleString()} rows`);
  } else if (err.error === "nesting_too_deep") {
    lines.push(`limit: ${err.limit}`, `detected_at_row: ${err.detected_at_row}`);
  } else if (err.error === "encoding_error") {
    lines.push(`byte_offset: ${err.byte_offset ?? "unknown"}`);
  } else if (err.error === "csv_parse_failed") {
    lines.push(`row: ${err.row}`, `column: ${err.column ?? "unknown"}`);
  } else if (err.error === "json_parse_failed") {
    lines.push(`byte_offset: ${err.byte_offset ?? "unknown"}`);
  } else if (err.error === "invalid_input") {
    lines.push(err.message);
  }
  errorDiv.textContent = lines.join("\n");
  errorDiv.classList.add("visible");
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;")
          .replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function typeBadge(t: ColumnMeta["inferred_type"]): string {
  return `<span class="type-badge type-${t}">${t}</span>`;
}

function fmtNum(n: number | null | undefined): string {
  return n == null ? "—" : n.toLocaleString(undefined, { maximumFractionDigits: 6 });
}

function renderResult(result: SchemaResult): void {
  if (result.truncated) truncBanner.classList.add("visible");

  const chunkNote = result.chunk_count > 1 ? `
    <div class="info-note">
      📦 Processed in ${result.chunk_count} chunks —
      cardinality estimates are upper bounds across chunk boundaries.
    </div>` : "";

  const summary = `
    <div class="summary">
      <span><strong>format</strong> ${result.detected_format}</span>
      <span><strong>rows</strong> ${result.row_count.toLocaleString()}</span>
      <span><strong>columns</strong> ${result.columns.length}</span>
      <span><strong>chunks</strong> ${result.chunk_count}</span>
      <span><strong>schemasniff</strong> v${result.schemasniff_version}</span>
    </div>${chunkNote}`;

  const rows = result.columns.map((col) => `
    <tr>
      <td>${col.index}</td>
      <td style="font-weight:600">${escapeHtml(col.name)}</td>
      <td>${typeBadge(col.inferred_type)}</td>
      <td>${col.null_count.toLocaleString()}</td>
      <td>${(col.null_ratio * 100).toFixed(1)}%</td>
      <td>${col.cardinality_estimate.toLocaleString()}</td>
      <td>${fmtNum(col.numeric_min)}</td>
      <td>${fmtNum(col.numeric_max)}</td>
    </tr>`).join("");

  outputDiv.innerHTML = summary + `
    <table>
      <thead><tr>
        <th>#</th><th>name</th><th>type</th><th>nulls</th>
        <th>null %</th><th>cardinality</th><th>min</th><th>max</th>
      </tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
}

// ── Core ──────────────────────────────────────────────────────────────────────

async function processFile(file: File): Promise<void> {
  showLoading();

  let text: string;
  try {
    text = await readFileAsText(file);
  } catch {
    hideLoading();
    renderError({ error: "encoding_error", byte_offset: null });
    return;
  }

  const t0 = performance.now();
  const result = await inferSchema(text, { onProgress: setProgress });
  const t1 = performance.now();

  console.table({
    file_size_mb:    (new TextEncoder().encode(text).length / 1024 / 1024).toFixed(2),
    detected_format: isSchemaError(result) ? "error" : result.detected_format,
    rows:            isSchemaError(result) ? "error" : result.row_count,
    truncated:       isSchemaError(result) ? "error" : result.truncated,
    chunks:          isSchemaError(result) ? "error" : result.chunk_count,
    total_ms:        Math.round(t1 - t0),
    ms_per_chunk:    isSchemaError(result)
                         ? "error"
                         : Math.round((t1 - t0) / result.chunk_count),
  });
  hideLoading();

  if (isSchemaError(result))       renderError(result);
  else if (isSchemaResult(result)) renderResult(result);
}

// ── Event wiring ──────────────────────────────────────────────────────────────

dropZone.addEventListener("dragover",  (e) => { e.preventDefault(); dropZone.classList.add("drag-over"); });
dropZone.addEventListener("dragleave", ()  => { dropZone.classList.remove("drag-over"); });
dropZone.addEventListener("drop", (e) => {
  e.preventDefault();
  dropZone.classList.remove("drag-over");
  const file = e.dataTransfer?.files[0];
  if (file) void processFile(file);
});

fileInput.addEventListener("change", () => {
  const file = fileInput.files?.[0];
  if (file) void processFile(file);
  fileInput.value = "";
});
