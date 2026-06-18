[![Sponsor](https://img.shields.io/badge/Sponsor-%E2%9D%A4-pink?logo=github)](https://github.com/sponsors/hyperpolymath)

# arghda-core

Lightweight proof-workspace manager for Agda. Extracted from
[`hyperpolymath/echo-types`](https://github.com/hyperpolymath/echo-types)
to its own repository on 2026-05-30 — see echo-types#159 for the move
record.

## v0.1 scope

- `Workspace` struct with four-state dir layout (`inbox`, `working`,
  `proven`, `rejected`)
- Filesystem watcher (`notify`-based)
- Linter rules:
  - `missing-safe-pragma` — file lacks `{-# OPTIONS --safe --without-K #-}`
  - `orphan-module` — `.agda` file not imported from `All.agda`
  - `unjustified-postulate` — `postulate` without an adjacent `-- JUSTIFY:` comment
- Workspace state machine — transitions are file moves, each logged to
  `.arghda/events.jsonl` (`claim`, `promote`, `reject`, `requeue`,
  `invalidate`)
- `dag` — emits the dependency-DAG JSON (nodes + import edges + blocked
  list) for a source tree: the contract a visual layer consumes
- `check` — runs Agda on a file and combines the typecheck verdict with the
  lint report (degrades gracefully when `agda` is absent)
- First-class import graph (the `graph` module, lifted out of the orphan rule)
- CLI (`arghda`): `init`, `scan`, `check`, `dag`, `claim`, `promote`,
  `reject`, `requeue`, `invalidate`, `events`, `watch`

Dogfooded against the echo-types corpus (193 modules): `dag` emits the
903-edge import graph; `scan` flags the known real orphan
(`experimental/echo-additive/VarianceGate.agda`) and the files deliberately
outside the `--safe --without-K` kernel cone.

Not yet: the remaining lint rules (`missing-without-k`, `unpinned-headline`,
`unused-import`, `tab-mix`), content-hash invalidation of `proven`, the
Groove service manifest, and the `.machine_readable/` RSR retrofit.

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
