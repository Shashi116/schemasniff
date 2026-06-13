import { readFileSync, writeFileSync, copyFileSync, existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root      = join(__dirname, "..");
const pkgDir    = join(root, "pkg");
const distDir   = join(root, "dist");

// ── 1. Merge package-override.json into pkg/package.json ─────────────────────
const generated = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));
const override  = JSON.parse(readFileSync(join(root,   "package-override.json"), "utf8"));
const merged    = { ...generated, ...override };
writeFileSync(join(pkgDir, "package.json"), JSON.stringify(merged, null, 2) + "\n");
console.log("✓ package.json merged");

// ── 2. Copy compiled TS files from dist/ into pkg/ ────────────────────────────
const copies = ["index.js", "index.d.ts", "types.js", "types.d.ts"];

for (const file of copies) {
    const src  = join(distDir, file);
    const dest = join(pkgDir,  file);
    if (existsSync(src)) {
        copyFileSync(src, dest);
        console.log(`✓ ${file} → pkg/`);
    } else {
        console.error(`✗ missing: dist/${file} — did you run tsc?`);
        process.exit(1);
    }
}

// ── 3. Verify final pkg/ contents ────────────────────────────────────────────
const required = [
    "schemasniff_bg.wasm",
    "schemasniff_bg.wasm.d.ts",
    "schemasniff.js",
    "schemasniff.d.ts",
    "index.js",
    "index.d.ts",
    "types.js",
    "types.d.ts",
    "package.json",
];

let ok = true;
for (const file of required) {
    if (!existsSync(join(pkgDir, file))) {
        console.error(`✗ pkg/${file} missing from final output`);
        ok = false;
    }
}

if (!ok) process.exit(1);
console.log("✓ all required files present in pkg/");
console.log("✓ postbuild complete — ready for: cd pkg && npm publish");
