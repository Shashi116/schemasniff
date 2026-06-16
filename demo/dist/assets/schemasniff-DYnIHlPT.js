let Y, q;
let __tla = (async () => {
  const A = "/assets/schemasniff_bg-CJJzh3CO.wasm", T = async (e = {}, t) => {
    let n;
    if (t.startsWith("data:")) {
      const _ = t.replace(/^data:.*?base64,/, "");
      let r;
      if (typeof Buffer == "function" && typeof Buffer.from == "function") r = Buffer.from(_, "base64");
      else if (typeof atob == "function") {
        const i = atob(_);
        r = new Uint8Array(i.length);
        for (let o = 0; o < i.length; o++) r[o] = i.charCodeAt(o);
      } else throw new Error("Cannot decode base64-encoded data URL");
      n = await WebAssembly.instantiate(r, e);
    } else {
      const _ = await fetch(t), r = _.headers.get("Content-Type") || "";
      if ("instantiateStreaming" in WebAssembly && r.startsWith("application/wasm")) n = await WebAssembly.instantiateStreaming(_, e);
      else {
        const i = await _.arrayBuffer();
        n = await WebAssembly.instantiate(i, e);
      }
    }
    return n.instance.exports;
  };
  Y = function(e) {
    const t = x(e, a.__wbindgen_malloc, a.__wbindgen_realloc), n = l, _ = a.infer_schema(t, n);
    if (_[2]) throw y(_[1]);
    return y(_[0]);
  };
  q = function() {
    a.on_wasm_init();
  };
  function E(e, t) {
    return Error(m(e, t));
  }
  function S(e, t) {
    const n = String(t), _ = x(n, a.__wbindgen_malloc, a.__wbindgen_realloc), r = l;
    h().setInt32(e + 4 * 1, r, true), h().setInt32(e + 4 * 0, _, true);
  }
  function M(e, t) {
    throw new Error(m(e, t));
  }
  function W() {
    return new Array();
  }
  function B() {
    return new Object();
  }
  function C(e, t, n) {
    e[t] = n;
  }
  function D(e, t, n) {
    e[t >>> 0] = n;
  }
  function U(e) {
    return e;
  }
  function O(e, t) {
    return m(e, t);
  }
  function I(e) {
    return BigInt.asUintN(64, e);
  }
  function v() {
    const e = a.__wbindgen_externrefs, t = e.grow(4);
    e.set(0, void 0), e.set(t + 0, void 0), e.set(t + 1, null), e.set(t + 2, true), e.set(t + 3, false);
  }
  let s = null;
  function h() {
    return (s === null || s.buffer.detached === true || s.buffer.detached === void 0 && s.buffer !== a.memory.buffer) && (s = new DataView(a.memory.buffer)), s;
  }
  function m(e, t) {
    return R(e >>> 0, t);
  }
  let d = null;
  function u() {
    return (d === null || d.byteLength === 0) && (d = new Uint8Array(a.memory.buffer)), d;
  }
  function x(e, t, n) {
    if (n === void 0) {
      const c = b.encode(e), f = t(c.length, 1) >>> 0;
      return u().subarray(f, f + c.length).set(c), l = c.length, f;
    }
    let _ = e.length, r = t(_, 1) >>> 0;
    const i = u();
    let o = 0;
    for (; o < _; o++) {
      const c = e.charCodeAt(o);
      if (c > 127) break;
      i[r + o] = c;
    }
    if (o !== _) {
      o !== 0 && (e = e.slice(o)), r = n(r, _, _ = o + e.length * 3, 1) >>> 0;
      const c = u().subarray(r + o, r + _), f = b.encodeInto(e, c);
      o += f.written, r = n(r, _, o, 1) >>> 0;
    }
    return l = o, r;
  }
  function y(e) {
    const t = a.__wbindgen_externrefs.get(e);
    return a.__externref_table_dealloc(e), t;
  }
  let w = new TextDecoder("utf-8", {
    ignoreBOM: true,
    fatal: true
  });
  w.decode();
  const L = 2146435072;
  let g = 0;
  function R(e, t) {
    return g += t, g >= L && (w = new TextDecoder("utf-8", {
      ignoreBOM: true,
      fatal: true
    }), w.decode(), g = t), w.decode(u().subarray(e, e + t));
  }
  const b = new TextEncoder();
  "encodeInto" in b || (b.encodeInto = function(e, t) {
    const n = b.encode(e);
    return t.set(n), {
      read: e.length,
      written: n.length
    };
  });
  let l = 0, a;
  function j(e) {
    a = e;
  }
  URL = globalThis.URL;
  const F = await T({
    "./schemasniff_bg.js": {
      __wbg_set_da33c120a6584674: D,
      __wbg_set_6be42768c690e380: C,
      __wbg_String_8564e559799eccda: S,
      __wbg_new_0b303268aa395a38: W,
      __wbg_new_20b778a4c5c691c3: B,
      __wbg___wbindgen_throw_bbadd78c1bac3a77: M,
      __wbg_Error_9dc85fe1bc224456: E,
      __wbindgen_init_externref_table: v,
      __wbindgen_cast_0000000000000001: U,
      __wbindgen_cast_0000000000000002: O,
      __wbindgen_cast_0000000000000003: I
    }
  }, A), { memory: V, infer_schema: $, on_wasm_init: k, __wbindgen_malloc: z, __wbindgen_realloc: J, __wbindgen_externrefs: N, __externref_table_dealloc: P, __wbindgen_start: p } = F, X = Object.freeze(Object.defineProperty({
    __proto__: null,
    __externref_table_dealloc: P,
    __wbindgen_externrefs: N,
    __wbindgen_malloc: z,
    __wbindgen_realloc: J,
    __wbindgen_start: p,
    infer_schema: $,
    memory: V,
    on_wasm_init: k
  }, Symbol.toStringTag, {
    value: "Module"
  }));
  j(X);
  p();
})();
export {
  __tla,
  Y as infer_schema,
  q as on_wasm_init
};
