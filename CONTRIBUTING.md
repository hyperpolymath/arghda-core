<!-- SPDX-License-Identifier: MPL-2.0 -->
# Contributing to arghda-core

arghda-core is the Rust engine for the arghda proof-workspace tool.

## Ground rules

- **arghda never proves.** Agda (directly, or via echidna) proves; arghda
  organises, lints, and emits the DAG / event JSON. Don't add a prover or
  claim a result Agda didn't return.
- **Licence:** MPL-2.0. New files carry an SPDX header from birth. Don't
  bulk-rewrite existing headers.
- **Green gate.** Every change keeps `just check`
  (`cargo fmt --check` + `clippy -D warnings` + `build` + `test`) green,
  and adds tests for new behaviour.

## Workflow

1. Branch from `main`.
2. Make the change; `just check` must pass.
3. Open a PR. CI runs build / test / clippy / fmt.

## Layout

- `src/` — the library plus the thin `arghda` CLI (`src/main.rs`).
- `src/lint/` — one module per lint rule (`LintRule` trait is the
  extension point).
- `src/graph.rs` — the first-class Agda import graph.
- `tests/` — integration tests; `tests/fixtures/` — tiny Agda fixtures.
- `docs/` — `.adoc` spec (`arghda-spec.adoc`) and vision/roadmap
  (`arghda-vision.adoc`).
- `.machine_readable/6a2/` — RSR machine-readable state; update
  `STATE.a2ml` as work lands.
