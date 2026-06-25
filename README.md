<!--
SPDX-License-Identifier: CC-BY-SA-4.0
SPDX-FileCopyrightText: 2025-2026 Jonathan D.A. Jewell <j.d.a.jewell@open.ac.uk>
-->

[![Sponsor](https://img.shields.io/badge/Sponsor-%E2%9D%A4-pink?logo=github)](https://github.com/sponsors/hyperpolymath)
[![License](https://img.shields.io/badge/License-MPL_2.0-blue.svg)](https://www.mozilla.org/MPL/2.0/)

Lightweight proof-workspace manager for Agda — the language-agnostic
engine half of **arghda**. Extracted from
[`hyperpolymath/echo-types`](https://github.com/hyperpolymath/echo-types)
to its own repository on 2026-05-30 (see echo-types#159 for the move
record).

arghda-core organises, lints, and reports on an Agda proof workspace; it
**never proves anything itself** — Agda does. It manages a four-state
triage workspace (inbox → working → proven → rejected) as file moves,
runs a lint pack targeting the *silent-failure* class (cases where Agda
appears to succeed but the file is not actually in the verified suite),
builds a first-class import-graph DAG, and emits a JSON + event-stream
contract a visual layer consumes.

# Status

v0.1 is feature-complete against `docs/arghda-spec.adoc`: all seven v0
Agda lint rules, the four-state workspace state machine, the
import-graph `dag`, the `check` command (Agda typecheck + lint verdict),
content-hash invalidation of `proven`, and `.arghda/config.toml`
operator configuration. The live status of record is
[`STATE.a2ml`](.machine_readable/6a2/STATE.a2ml).

# Lint rules

Hard-block (forbids the `working` `→` `proven` transition):

- `missing-safe-pragma` — file lacks `{-#` `OPTIONS` `--safe`
  `--without-K` `#-}`.

- `orphan-module` — `.agda` file unreachable from any CI root (roots are
  auto-discovered as `All.agda`/`Smoke.agda`, or passed via `--entry`;
  reachability is the *union*, so a module verified from any root is not
  an orphan).

- `unjustified-postulate` — `postulate` without an adjacent `--`
  `JUSTIFY:` comment.

Warn (surfaced, non-blocking):

- `escape-hatch` — termination overrides (`TERMINATING`,
  `NON_TERMINATING`, `NO_TERMINATION_CHECK`) and trust primitives
  (`believe_me`/`primTrustMe`).

- `tab-mix` — a tab in leading whitespace.

- `unpinned-headline` — a top-level theorem whose name matches the
  headline pattern is not pinned in any `Smoke.agda` via a `using` `(`
  `…` `)` clause (the estate "every headline pinned in Smoke"
  discipline). The pattern is operator-configurable; its default
  `^[a-z][A-Za-z0-9-]*$` is deliberately broad, so operators narrow it
  to their own convention. Self-skips when no `Smoke.agda` is in scope.

- `unused-import` — re-emits the findings of the external
  [`agda-unused`](https://github.com/msuperdock/agda-unused) tool.
  Opt-in behind `scan` `--unused`; invoked per file in local mode with
  `LC_ALL=C.UTF-8`; skipped with a note if the binary is not on `PATH`.

# Commands

The CLI is `arghda`.

| Command | What it does |
|----|----|
| `init` | Create the four-state workspace layout at a path. |
| `scan` | Lint every `.agda` file under a path. `--unused` adds the agda-unused pass; `--config`/`--headline-pattern` tune configuration. |
| `check` | Run Agda on one file and lint it; combined verdict (degrades when `agda` is absent). |
| `dag` | Emit the dependency-DAG JSON — nodes (lint status + declared `headlines`), import edges, and a blocked list. |
| `claim` / `promote` / `reject` / `requeue` / `invalidate` | State-machine transitions; each is a file move logged to `.arghda/events.jsonl`. |
| `events` | Replay the workspace event log. |
| `stale` | List `proven` files whose content changed since promotion; `--invalidate` returns them to inbox. |
| `watch` | Watch `inbox/` and `working/` and print events. |

# Configuration

`scan` and `dag` read `.arghda/config.toml` (from
`<PATH>/.arghda/config.toml`, or an explicit `--config` `<file>`).
Precedence is built-in default \< `config.toml` \< CLI flag.

```toml
[lint]
headline_pattern = "^[a-z][A-Za-z0-9-]*$"
```

# Build

```sh
just check    # fmt-check + clippy (-D warnings) + build + test — the CI gate
cargo build
cargo test
```

`agda` and `agda-unused` are optional: the `check` and `scan` `--unused`
paths degrade gracefully (with a note) when the binary is absent, so the
rest of the engine still works in an Agda-less environment.

# Smoke against an Agda workspace

```sh
cargo run -- scan path/to/your/agda/sources
```

Dogfooded against the echo-types corpus (193 modules): `dag` emits the
903-edge import graph; multi-root discovery (`All.agda`, `Smoke.agda`,
`Ordinal/Buchholz/Smoke.agda`, `characteristic/All.agda`,
`examples/All.agda`) narrows orphan reports from 38 to the 17 genuine
orphans.

# Ecosystem

Part of the [hyperpolymath ecosystem](https://github.com/hyperpolymath).
arghda splits into `arghda-core` (this engine) and the planned
`arghda-studio` / `arghda-panll` visual layers, which consume the `dag`
JSON + `events.jsonl` contract. The motivating workspace was the
echo-types proof pipeline, but arghda-core has no echo-types-specific
code.

Adjacent projects:

- [echo-types](https://github.com/hyperpolymath/echo-types) — the Agda
  library that motivated arghda’s design (not a build dependency).

- [absolute-zero](https://github.com/hyperpolymath/absolute-zero) — a
  sister Agda library under the same `--safe` `--without-K` discipline.

# Machine-readable

Per the Rhodium Standard, structured project metadata lives under
[`.machine_readable/6a2/`](.machine_readable/6a2/) — `STATE`, `META`,
`ECOSYSTEM`, `AGENTIC`, `NEUROSYM`, `PLAYBOOK` in A2ML — with
[`0-AI-MANIFEST.a2ml`](0-AI-MANIFEST.a2ml) as the AI entry point and
[`EXPLAINME.adoc`](EXPLAINME.adoc) as the orientation pointer.

# Licence

Code, configuration and scripts are `MPL-2.0` (see [LICENSE](LICENSE));
prose documentation is `CC-BY-SA-4.0`, per the [hyperpolymath licence
policy](https://github.com/hyperpolymath/standards) (Rule 1). This split
is a **formal invariant**: every tracked file carries the appropriate
SPDX header, checked by `scripts/check-spdx.sh` in `just` `check` and in
CI. Third-party, generated, and test-data files are explicitly excluded
([`licensing-policy.toml`](.machine_readable/licensing-policy.toml)) and
are never relicensed — vendoring third-party source in-tree fails the
check until it is listed as excluded with its original SPDX preserved.
