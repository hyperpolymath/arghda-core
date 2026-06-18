<!-- SPDX-License-Identifier: MPL-2.0 -->
# Security Policy

## Supported versions

arghda-core is alpha (0.1.x). Only the latest `main` is supported.

## Reporting a vulnerability

Please report suspected security issues **privately** to
**j.d.a.jewell@open.ac.uk**. Do not open a public issue for a suspected
vulnerability.

Where possible, include:

- a description of the issue and its impact;
- steps or a proof-of-concept to reproduce;
- the commit or version affected.

## Response

We aim to acknowledge a report within 7 days and to agree a disclosure
timeline with the reporter.

## Scope

This policy covers the arghda-core engine. arghda reads `.agda` files as
text and shells out to the configured `agda` (and, in future,
`agda-unused`) binary on files you point it at; it never executes proof
code itself. Reports about untrusted-input handling — path traversal,
command construction, denial of service on malformed input — are
especially welcome.
