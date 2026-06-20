#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
#
# Licence invariant for arghda-core (enforced by `just check` + Rust CI).
#
#   code / config / scripts / state   ->  SPDX-License-Identifier: MPL-2.0
#   prose documentation (*.adoc, *.md) ->  SPDX-License-Identifier: CC-BY-SA-4.0
#
# Files that are NOT ours, generated, or test-input data are EXCLUDED and must
# never carry the repo's licence choices. The machine-readable declaration of
# this rule is .machine_readable/licensing-policy.toml; the split itself comes
# from hyperpolymath/standards LICENCE-POLICY.adoc Rule 1.
#
# Guard for third-party code: any tracked, non-excluded file that lacks our
# header FAILS the check. Vendoring third-party source therefore forces a
# conscious exclusion (and the file keeps its original SPDX) rather than being
# silently relicensed.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

MPL='SPDX-License-Identifier: MPL-2.0'
CC='SPDX-License-Identifier: CC-BY-SA-4.0'
fail=0

# Not ours / generated / test-input data — see [excluded] in the policy file.
excluded() {
  case "$1" in
    Cargo.lock | LICENSE) return 0 ;;          # generated lockfile / the licence text itself
    target/* | tests/fixtures/*) return 0 ;;   # build output / Agda test-input data
    *) return 1 ;;
  esac
}

while IFS= read -r f; do
  excluded "$f" && continue
  case "$f" in
    *.adoc | *.md) want="$CC"; label="CC-BY-SA-4.0 (prose)" ;;
    *) want="$MPL"; label="MPL-2.0 (code/config/script/state)" ;;
  esac
  if ! grep -qF "$want" "$f"; then
    echo "  MISSING $label: $f"
    fail=1
  fi
done < <(git ls-files)

if [ "$fail" -ne 0 ]; then
  cat >&2 <<'MSG'
SPDX licence invariant: FAIL (see above).
Fix one of:
  * add the correct SPDX header to the file, OR
  * if the file is third-party / generated / test data, add it to the
    excluded set in scripts/check-spdx.sh AND
    .machine_readable/licensing-policy.toml — never relicense others' code.
MSG
  exit 1
fi
echo "SPDX licence invariant: OK"
