# arghda-core

Lightweight proof-workspace manager for Agda. Extracted from
[`hyperpolymath/echo-types`](https://github.com/hyperpolymath/echo-types)
to its own repository on 2026-05-30 — see echo-types#159 for the move
record.

## v0.1 scope

- `Workspace` struct with four-state dir layout (`inbox`, `working`,
  `proven`, `rejected`)
- Filesystem watcher (`notify`-based)
- Two linter rules:
  - `missing-safe-pragma` — file lacks `{-# OPTIONS --safe --without-K #-}`
  - `orphan-module` — `.agda` file not imported from `All.agda`
- CLI (`arghda`) with subcommands: `init`, `scan`, `watch`

Not yet: `promote`, `reject`, `dag` (v0.1.x).

## Build

```
cargo build
cargo test
```

## Smoke against an Agda workspace

```
cargo run -- scan path/to/your/agda/sources
```

Expected output enumerates per-file lint hits. With no `All.agda`
present, `orphan-module` is a no-op; with one present, modules
unreachable from `All.agda` get flagged.

## Ecosystem context

Part of the [hyperpolymath ecosystem](https://github.com/hyperpolymath).
The original design motivation was the echo-types proof pipeline
(triage folders `inbox → working → proven → rejected`), but `arghda`
operates on any `--safe --without-K` Agda workspace; it has no
echo-types-specific code.

Adjacent projects:

- [echo-types](https://github.com/hyperpolymath/echo-types) — the
  Agda library that motivated arghda's design; arghda is not a
  build-dependency.
- [absolute-zero](https://github.com/hyperpolymath/absolute-zero) —
  a sister Agda library with the same `--safe --without-K` discipline.

## License

MPL-2.0. SPDX headers on each source file.
