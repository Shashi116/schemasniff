# Contributing to schemasniff

Thanks for considering a contribution. This project has a strong security focus —
please read the guidelines below before opening a PR.

## Before you start

- For bugs, open an issue first describing the problem with a minimal reproduction
- For features, open an issue to discuss the approach before writing code
- For security vulnerabilities, see [SECURITY.md](./SECURITY.md) — do not open a public issue

## Development setup

```bash
# Install Rust and the WASM target
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown

# Install wasm-pack
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

# Clone and build
git clone https://github.com/shashik116/schemasniff.git
cd schemasniff
npm install
npm run build
```

## Running tests

```bash
cargo test                          # Rust unit + integration tests
cargo clippy -- -D warnings         # Lint — must be zero warnings
cargo audit && cargo deny check     # Security checks
npm run lint                        # ESLint on TypeScript wrapper
npm run test:all                    # All of the above in one command
```

## Hard rules — these will block your PR if violated

1. **No `unsafe` code.** `#![forbid(unsafe_code)]` is permanent.
2. **No `.unwrap()` or `.expect()` in production code paths.** Use `.unwrap_or(...)` with a safe
   fallback, or propagate a `SchemaError` via `?`. Test code (`#[cfg(test)]`) is exempt.
3. **No new fields on `SchemaError` containing `String`, `Vec<u8>`, or any heap-allocated type
   that could carry cell content.** Only `usize` and `Option<usize>` are permitted. This is
   enforced by a compile-time size seal test — if your change breaks it, the build will not pass.
4. **No `console.log` / `console.warn` / `console.error` in `src/*.ts`.** Library code must be
   silent. Enforced by `eslint`'s `no-console` rule.
5. **No new runtime dependency without discussion.** Open an issue first. Every dependency added
   is one more thing `cargo audit` has to watch and one more supply-chain risk. We have actively
   reduced our dependency count (53 → ~40) and want to keep that trend, not reverse it.
6. **Hard caps (`MAX_ROWS`, `MAX_COLS`, `MAX_CELL_BYTES`, `MAX_JSON_DEPTH`) are not configurable.**
   They exist to prevent OOM/DoS in the browser. If you have a use case that needs higher limits,
   open an issue to discuss — do not just bump the constant in a PR.

## Adding a fuzz target

If you add a new parsing code path, add a corresponding fuzz target in `fuzz/fuzz_targets/`.
See existing targets for the pattern. Run it locally for at least 60 seconds before submitting:

```bash
cargo +nightly fuzz run fuzz_<your_target> -- -max_total_time=60
```

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add NDJSON streaming support
fix: correct null_ratio calculation for empty columns
docs: clarify cardinality estimate accuracy in README
ci: pin new GitHub Action to commit SHA
```

## Pull request checklist

- [ ] `npm run test:all` passes locally
- [ ] New code has unit tests
- [ ] New parsing logic has a corresponding fuzz target
- [ ] No new `unwrap()`/`expect()` in production code
- [ ] No new dependency without prior discussion in an issue
- [ ] `CHANGELOG.md` updated under an `[Unreleased]` section

## Code of conduct

Be respectful. Disagree on technical merit, not personality. Maintainers reserve the right
to close PRs or issues that violate this.
