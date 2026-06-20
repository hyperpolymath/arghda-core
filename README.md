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
  - `orphan-module` — `.agda` file not reachable from any CI root (roots
    are auto-discovered as `All.agda`/`Smoke.agda`, or passed via `--entry`;
    reachability is the *union*, so a module verified from any root is not
    an orphan)
  - `unjustified-postulate` — `postulate` without an adjacent `-- JUSTIFY:` comment
  - `escape-hatch` (warn) — termination overrides (`TERMINATING`,
    `NON_TERMINATING`, `NO_TERMINATION_CHECK`) and trust primitives
    (`believe_me`/`primTrustMe`)
  - `tab-mix` (warn) — a tab in leading whitespace
  - `unpinned-headline` (warn) — a top-level theorem whose name matches the
    headline pattern is not pinned in any `Smoke.agda` via a `using ( … )`
    clause (the estate "every headline pinned in Smoke" discipline). The
    pattern is operator-configurable (via `.arghda/config.toml` or
    `--headline-pattern <regex>`); its default `^[a-z][A-Za-z0-9-]*$` is
    deliberately broad, so operators narrow it to their own headline-naming
    convention. Self-skips when no `Smoke.agda` is in scope (e.g. a
    single-file `check`)
  - `unused-import` (warn) — re-emits the findings of the external
    [`agda-unused`](https://github.com/msuperdock/agda-unused) tool. Opt-in
    behind `scan --unused` (it runs `agda-unused` per file in local mode and
    re-checks each file); skipped with a note if the binary is not on `PATH`.
    Invoked with `LC_ALL=C.UTF-8` so it can read UTF-8 Agda sources
- Workspace state machine — transitions are file moves, each logged to
  `.arghda/events.jsonl` (`claim`, `promote`, `reject`, `requeue`,
  `invalidate`)
- `dag` — emits the dependency-DAG JSON (nodes — each with its lint status
  and declared `headlines` — plus import edges and a blocked list) for a
  source tree: the contract a visual layer consumes
- `check` — runs Agda on a file and combines the typecheck verdict with the
  lint report (degrades gracefully when `agda` is absent)
- First-class import graph (the `graph` module, lifted out of the orphan rule)
- CLI (`arghda`): `init`, `scan`, `check`, `dag`, `claim`, `promote`,
  `reject`, `requeue`, `invalidate`, `events`, `watch`

Dogfooded against the echo-types corpus (193 modules): `dag` emits the
903-edge import graph; multi-root discovery (5 roots: `All.agda`,
`Smoke.agda`, `Ordinal/Buchholz/Smoke.agda`, `characteristic/All.agda`,
`examples/All.agda`) narrows orphan reports from 38 to the 17 genuine
orphans — the `experimental/echo-additive/` tree (including
`VarianceGate.agda`, the orphan the 2026-06-16 trust audit found by hand)
plus standalone scratch files. `scan` also flags the files deliberately
outside the `--safe --without-K` kernel cone (`Fidelity.agda`, the cubical
island, the postulated shadow).

Configuration: `scan` and `dag` read `.arghda/config.toml` (from
`<PATH>/.arghda/config.toml`, or an explicit `--config <file>`). Precedence is
built-in default < `config.toml` < CLI flag. Current schema:

```toml
[lint]
headline_pattern = "^[a-z][A-Za-z0-9-]*$"
```

Not yet: the Groove service manifest. (`missing-without-k` is subsumed by
`missing-safe-pragma`, which already reports a missing `--without-K`.)

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
