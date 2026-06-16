(async () => {
  (function() {
    const t = document.createElement("link").relList;
    if (t && t.supports && t.supports("modulepreload")) return;
    for (const n of document.querySelectorAll('link[rel="modulepreload"]')) r(n);
    new MutationObserver((n) => {
      for (const l of n) if (l.type === "childList") for (const u of l.addedNodes) u.tagName === "LINK" && u.rel === "modulepreload" && r(u);
    }).observe(document, {
      childList: true,
      subtree: true
    });
    function s(n) {
      const l = {};
      return n.integrity && (l.integrity = n.integrity), n.referrerPolicy && (l.referrerPolicy = n.referrerPolicy), n.crossOrigin === "use-credentials" ? l.credentials = "include" : n.crossOrigin === "anonymous" ? l.credentials = "omit" : l.credentials = "same-origin", l;
    }
    function r(n) {
      if (n.ep) return;
      n.ep = true;
      const l = s(n);
      fetch(n.href, l);
    }
  })();
  const C = "modulepreload", N = function(e) {
    return "/" + e;
  }, w = {}, O = function(t, s, r) {
    let n = Promise.resolve();
    if (s && s.length > 0) {
      document.getElementsByTagName("link");
      const u = document.querySelector("meta[property=csp-nonce]"), o = (u == null ? void 0 : u.nonce) || (u == null ? void 0 : u.getAttribute("nonce"));
      n = Promise.allSettled(s.map((i) => {
        if (i = N(i), i in w) return;
        w[i] = true;
        const c = i.endsWith(".css"), a = c ? '[rel="stylesheet"]' : "";
        if (document.querySelector(`link[href="${i}"]${a}`)) return;
        const d = document.createElement("link");
        if (d.rel = c ? "stylesheet" : C, c || (d.as = "script"), d.crossOrigin = "", d.href = i, o && d.setAttribute("nonce", o), document.head.appendChild(d), c) return new Promise((m, T) => {
          d.addEventListener("load", m), d.addEventListener("error", () => T(new Error(`Unable to preload CSS for ${i}`)));
        });
      }));
    }
    function l(u) {
      const o = new Event("vite:preloadError", {
        cancelable: true
      });
      if (o.payload = u, window.dispatchEvent(o), !o.defaultPrevented) throw u;
    }
    return n.then((u) => {
      for (const o of u || []) o.status === "rejected" && l(o.reason);
      return t().catch(l);
    });
  };
  function f(e) {
    return "error" in e;
  }
  function R(e) {
    return !("error" in e);
  }
  const b = 10 * 1024 * 1024, A = 4 * 1024 * 1024;
  let E = null, h = null;
  function D() {
    return h === null && (h = O(() => import("./schemasniff-DYnIHlPT.js").then(async (m) => {
      await m.__tla;
      return m;
    }), []).then((e) => {
      E = e;
    }).catch((e) => {
      throw h = null, e;
    })), h;
  }
  let g = false;
  function k(e) {
    try {
      const t = E.infer_schema(e);
      return typeof t == "object" && t !== null && "error" in t, t;
    } catch {
      return {
        error: "unrecognized_format"
      };
    }
  }
  function F(e) {
    const t = e.length;
    if ((t > b * 0.8 ? new TextEncoder().encode(e).length : t) <= b) return [
      e
    ];
    const r = new TextEncoder(), n = e.split(`
`), l = n[0] ?? "", u = r.encode(l).length, o = [], i = [
      l
    ];
    let c = u;
    for (let a = 1; a < n.length; a++) {
      const d = n[a] ?? "", m = r.encode(d).length + 1;
      c + m > A && i.length > 1 ? (o.push(i.join(`
`)), i.length = 0, i.push(l, d), c = u + 1 + m) : (i.push(d), c += m);
    }
    return i.length > 1 && o.push(i.join(`
`)), o.length > 0 ? o : [
      e
    ];
  }
  function j(e, t) {
    const s = e[0], r = /* @__PURE__ */ new Map();
    for (const o of s.columns) r.set(o.name, {
      null_count: o.null_count,
      numeric_min: o.numeric_min ?? null,
      numeric_max: o.numeric_max ?? null,
      cardinality_estimate: o.cardinality_estimate,
      inferred_type: o.inferred_type,
      nullable: o.nullable,
      index: o.index
    });
    let n = s.row_count, l = s.truncated;
    for (let o = 1; o < e.length; o++) {
      const i = e[o];
      n += i.row_count, i.truncated && (l = true);
      for (const c of i.columns) {
        const a = r.get(c.name);
        if (!a) r.set(c.name, {
          null_count: c.null_count,
          numeric_min: c.numeric_min ?? null,
          numeric_max: c.numeric_max ?? null,
          cardinality_estimate: c.cardinality_estimate,
          inferred_type: c.inferred_type,
          nullable: c.nullable,
          index: c.index
        });
        else {
          a.null_count += c.null_count, a.cardinality_estimate += c.cardinality_estimate, a.nullable = a.nullable || c.nullable;
          const d = c.numeric_min ?? null, m = c.numeric_max ?? null;
          d !== null && (a.numeric_min = a.numeric_min === null ? d : Math.min(a.numeric_min, d)), m !== null && (a.numeric_max = a.numeric_max === null ? m : Math.max(a.numeric_max, m)), a.inferred_type === "unknown" && c.inferred_type !== "unknown" && (a.inferred_type = c.inferred_type);
        }
      }
    }
    const u = Array.from(r.entries()).sort((o, i) => o[1].index - i[1].index).map(([o, i]) => ({
      name: o,
      index: i.index,
      inferred_type: i.inferred_type,
      nullable: i.nullable,
      null_count: i.null_count,
      null_ratio: n > 0 ? i.null_count / Number(n) : 0,
      numeric_min: i.numeric_min,
      numeric_max: i.numeric_max,
      cardinality_estimate: i.cardinality_estimate
    }));
    return {
      row_count: n,
      truncated: l,
      detected_format: s.detected_format,
      schemasniff_version: s.schemasniff_version,
      chunk_count: t,
      columns: u
    };
  }
  async function H(e, t) {
    const s = [];
    for (let r = 0; r < e.length; r++) {
      await new Promise((l) => setTimeout(l, 0));
      const n = k(e[r]);
      if (f(n)) return n;
      s.push(n), t == null ? void 0 : t((r + 1) / e.length);
    }
    return s.length === 0 ? {
      error: "empty_input"
    } : s.length === 1 ? {
      ...s[0],
      chunk_count: 1
    } : j(s, e.length);
  }
  async function U(e, t) {
    if (typeof e != "string") return {
      error: "invalid_input",
      message: "input must be a string"
    };
    if (g) return {
      error: "invalid_input",
      message: "concurrent calls are not supported"
    };
    g = true;
    try {
      await D();
      const s = F(e);
      if (s.length === 1) {
        const r = k(s[0]);
        return f(r) ? r : {
          ...r,
          chunk_count: 1
        };
      }
      return await H(s, t == null ? void 0 : t.onProgress);
    } catch {
      return {
        error: "unrecognized_format"
      };
    } finally {
      g = false;
    }
  }
  const _ = document.getElementById("drop-zone"), y = document.getElementById("file-input"), B = document.getElementById("loading"), v = document.getElementById("progress-bar"), S = document.getElementById("progress-label"), P = document.getElementById("truncation-banner"), p = document.getElementById("error"), M = document.getElementById("output");
  async function q(e) {
    const t = await e.arrayBuffer();
    return new TextDecoder("utf-8", {
      fatal: true
    }).decode(t);
  }
  function z() {
    v.style.width = "0%", S.textContent = "\u23F3 Analysing schema\u2026", B.classList.add("visible"), P.classList.remove("visible"), p.classList.remove("visible"), p.textContent = "", M.innerHTML = "";
  }
  function x() {
    B.classList.remove("visible"), v.style.width = "0%";
  }
  function K(e) {
    const t = Math.round(e * 100);
    v.style.width = `${t}%`, S.textContent = `\u23F3 Analysing\u2026 ${t}%`;
  }
  function L(e) {
    const t = [
      `error: ${e.error}`
    ];
    e.error === "input_too_large" ? (t.push(`limit:  ${(e.limit_bytes / 1024 / 1024).toFixed(1)} MB`), t.push(`actual: ${(e.actual_bytes / 1024 / 1024).toFixed(2)} MB`)) : e.error === "too_many_columns" ? t.push(`limit: ${e.limit}`, `actual: ${e.actual}`) : e.error === "row_limit_reached" ? t.push(`limit: ${e.limit.toLocaleString()} rows`) : e.error === "nesting_too_deep" ? t.push(`limit: ${e.limit}`, `detected_at_row: ${e.detected_at_row}`) : e.error === "encoding_error" ? t.push(`byte_offset: ${e.byte_offset ?? "unknown"}`) : e.error === "csv_parse_failed" ? t.push(`row: ${e.row}`, `column: ${e.column ?? "unknown"}`) : e.error === "json_parse_failed" ? t.push(`byte_offset: ${e.byte_offset ?? "unknown"}`) : e.error === "invalid_input" && t.push(e.message), p.textContent = t.join(`
`), p.classList.add("visible");
  }
  function W(e) {
    return e.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
  function Z(e) {
    return `<span class="type-badge type-${e}">${e}</span>`;
  }
  function $(e) {
    return e == null ? "\u2014" : e.toLocaleString(void 0, {
      maximumFractionDigits: 6
    });
  }
  function V(e) {
    e.truncated && P.classList.add("visible");
    const t = e.chunk_count > 1 ? `
    <div class="info-note">
      \u{1F4E6} Processed in ${e.chunk_count} chunks \u2014
      cardinality estimates are upper bounds across chunk boundaries.
    </div>` : "", s = `
    <div class="summary">
      <span><strong>format</strong> ${e.detected_format}</span>
      <span><strong>rows</strong> ${e.row_count.toLocaleString()}</span>
      <span><strong>columns</strong> ${e.columns.length}</span>
      <span><strong>chunks</strong> ${e.chunk_count}</span>
      <span><strong>schemasniff</strong> v${e.schemasniff_version}</span>
    </div>${t}`, r = e.columns.map((n) => `
    <tr>
      <td>${n.index}</td>
      <td style="font-weight:600">${W(n.name)}</td>
      <td>${Z(n.inferred_type)}</td>
      <td>${n.null_count.toLocaleString()}</td>
      <td>${(n.null_ratio * 100).toFixed(1)}%</td>
      <td>${n.cardinality_estimate.toLocaleString()}</td>
      <td>${$(n.numeric_min)}</td>
      <td>${$(n.numeric_max)}</td>
    </tr>`).join("");
    M.innerHTML = s + `
    <table>
      <thead><tr>
        <th>#</th><th>name</th><th>type</th><th>nulls</th>
        <th>null %</th><th>cardinality</th><th>min</th><th>max</th>
      </tr></thead>
      <tbody>${r}</tbody>
    </table>`;
  }
  async function I(e) {
    z();
    let t;
    try {
      t = await q(e);
    } catch {
      x(), L({
        error: "encoding_error",
        byte_offset: null
      });
      return;
    }
    const s = performance.now(), r = await U(t, {
      onProgress: K
    }), n = performance.now();
    console.table({
      file_size_mb: (new TextEncoder().encode(t).length / 1024 / 1024).toFixed(2),
      detected_format: f(r) ? "error" : r.detected_format,
      rows: f(r) ? "error" : r.row_count,
      truncated: f(r) ? "error" : r.truncated,
      chunks: f(r) ? "error" : r.chunk_count,
      total_ms: Math.round(n - s),
      ms_per_chunk: f(r) ? "error" : Math.round((n - s) / r.chunk_count)
    }), x(), f(r) ? L(r) : R(r) && V(r);
  }
  _.addEventListener("dragover", (e) => {
    e.preventDefault(), _.classList.add("drag-over");
  });
  _.addEventListener("dragleave", () => {
    _.classList.remove("drag-over");
  });
  _.addEventListener("drop", (e) => {
    var _a;
    e.preventDefault(), _.classList.remove("drag-over");
    const t = (_a = e.dataTransfer) == null ? void 0 : _a.files[0];
    t && I(t);
  });
  y.addEventListener("change", () => {
    var _a;
    const e = (_a = y.files) == null ? void 0 : _a[0];
    e && I(e), y.value = "";
  });
})();
