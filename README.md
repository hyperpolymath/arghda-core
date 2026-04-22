# arghda-core

Lightweight proof-workspace manager for Agda. This crate parks inside
`echo-types` until its own repo exists; see `docs/arghda-spec.adoc` at
the echo-types root for the design.

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
cd arghda-core
cargo build
cargo test
```

## Smoke against echo-types

```
cd arghda-core
cargo run -- scan ../proofs/agda
```

Expected: every `.agda` file passes `missing-safe-pragma`; `orphan-module`
checks against `../proofs/agda/All.agda`.
