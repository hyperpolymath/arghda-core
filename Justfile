# SPDX-License-Identifier: MPL-2.0
# Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
#
# Task runner for arghda-core. See https://just.systems
set shell := ["bash", "-uc"]

# List recipes.
default:
    @just --list --unsorted

# Build everything (lib + bin + tests).
build:
    cargo build --all-targets

# Release CLI binary (target/release/arghda).
build-release:
    cargo build --release

# Unit + integration tests.
test:
    cargo test

# Format in place.
fmt:
    cargo fmt

# Format check (CI gate).
fmt-check:
    cargo fmt --check

# Clippy with warnings as errors (CI gate).
lint:
    cargo clippy --all-targets -- -D warnings

# The full CI gate, exactly what .github/workflows/rust-ci.yml runs.
check: fmt-check lint build test

# Alias for CI.
ci: check

# Lint an Agda source tree (auto-discovers All.agda/Smoke.agda roots).
scan path:
    cargo run -- scan "{{path}}"

# Emit the dependency-DAG JSON for an Agda source tree.
dag path:
    cargo run -- dag "{{path}}"

# Typecheck one file with Agda + lint it (needs `agda` on PATH).
agda-check file:
    cargo run -- check "{{file}}"

# RSR: the mandated machine-readable artefacts are present and well-formed.
validate-rsr:
    @for f in STATE META ECOSYSTEM AGENTIC NEUROSYM PLAYBOOK; do \
        test -f ".machine_readable/6a2/$f.a2ml" || { echo "missing .machine_readable/6a2/$f.a2ml"; exit 1; }; \
    done
    @grep -q 'scoping-first = true' .machine_readable/6a2/META.a2ml || { echo "META.a2ml missing maintenance axes"; exit 1; }
    @test -f 0-AI-MANIFEST.a2ml || { echo "missing 0-AI-MANIFEST.a2ml"; exit 1; }
    @echo "RSR artefacts present and well-formed"

# Full local gate: RSR artefacts + the CI gate.
validate: validate-rsr check
