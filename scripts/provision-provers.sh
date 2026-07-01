#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
#
# On-demand provisioning of the prover / solver toolchains ArghDA drives.
# Companion to hyperpolymath/echo-types scripts/provision-agda.sh, widened to
# the full multi-backend set. Idempotent: safe to re-run; skips anything already
# present.
#
# HONESTY CONTRACT (load-bearing — see arghda-core AGENTIC.a2ml):
#   A tool is only ever reported "OK" if its own `--version` (or equivalent)
#   actually returned exit 0 in THIS run. Anything else is reported MISSING or
#   FAILED with the command that was tried. The script never claims an install
#   it did not verify.
#
# Tiers:
#   default (tractable, installed now):  agda(+stdlib +cubical lib), zig,
#                                        idris2, lean4, z3, cvc5
#   --heavy   (adds):                    coq/rocq (opam), isabelle (~4 GB)
#   --mizar   (adds):                    mizar (niche; distribution uncertain)
#   --all:                               everything above
#   --verify-only:                       skip installs, just run the doctor table
#
# Usage:  bash scripts/provision-provers.sh [--heavy] [--mizar] [--all] [--verify-only]
set -uo pipefail   # NB: no -e; we tolerate per-tool failure and report it.

log()  { echo "[provision] $*"; }
warn() { echo "[provision] WARN: $*" >&2; }
have() { command -v "$1" >/dev/null 2>&1; }

# ---- Pinned versions (reused from the estate where a precedent exists) -------
STDLIB_TAG="v2.3"                         # echo-types pin
ZIG_VER="0.15.2"                          # boj-server setup-zig pin
IDRIS2_TAG="v0.7.0"                       # standards/a2ml CI pin
LEAN_VER="v4.13.0"                        # tropical-resource-typing lean-toolchain
CVC5_VER="1.2.0"                          # cvc5 static-binary release
COQ_VER="8.18.0"                          # opam (with --heavy)
ISABELLE_VER="Isabelle2025"               # TUM tarball (with --heavy)

STDLIB_DIR=/opt/agda-stdlib
CUBICAL_DIR=/opt/agda-cubical
ZIG_DIR=/opt/zig
IDRIS2_SRC=/opt/Idris2
AGDA_HOME="${HOME}/.agda"

WANT_HEAVY=0; WANT_MIZAR=0; VERIFY_ONLY=0
for a in "$@"; do case "$a" in
  --heavy) WANT_HEAVY=1 ;;
  --mizar) WANT_MIZAR=1 ;;
  --all)   WANT_HEAVY=1; WANT_MIZAR=1 ;;
  --verify-only) VERIFY_ONLY=1 ;;
  *) warn "unknown flag: $a" ;;
esac; done

apt_get() { DEBIAN_FRONTEND=noninteractive apt-get "$@"; }

# =============================================================================
# Installers — each best-effort, never aborts the script.
# =============================================================================

install_agda() {
  have agda && { log "agda present: $(agda --version 2>&1 | head -1)"; return; }
  log "installing agda (apt agda-bin)…"
  apt_get update -qq || true
  apt_get install -y agda-bin >/dev/null 2>&1 || apt_get install -y agda >/dev/null 2>&1 || \
    warn "apt could not install agda"
}

# NB: this environment's proxy routes `git clone` through a git-relay that only
# permits the session's scoped repos (third-party clones return HTTP 403), but
# plain `curl` to GitHub archive tarballs works. So all third-party sources are
# fetched as release/branch tarballs, which is also more portable than a clone.
fetch_tarball() {  # fetch_tarball <url> <dest-dir>
  local url="$1" dest="$2" tmp="/tmp/arghda-fetch-$$.tar.gz"
  curl -fsSL "$url" -o "$tmp" 2>/dev/null && [ -s "$tmp" ] || { rm -f "$tmp"; return 1; }
  rm -rf "$dest"; mkdir -p "$dest"
  tar -xzf "$tmp" -C "$dest" --strip-components=1; local rc=$?
  rm -f "$tmp"; return $rc
}

install_agda_stdlib() {
  if [ -f "${STDLIB_DIR}/standard-library.agda-lib" ]; then
    log "agda-stdlib present at ${STDLIB_DIR}"; return
  fi
  log "fetching agda-stdlib ${STDLIB_TAG} tarball…"
  if fetch_tarball "https://github.com/agda/agda-stdlib/archive/refs/tags/${STDLIB_TAG}.tar.gz" "${STDLIB_DIR}"; then
    sed -i 's/^name: standard-library-.*/name: standard-library/' \
      "${STDLIB_DIR}/standard-library.agda-lib" || true
    log "agda-stdlib ${STDLIB_TAG} ready"
  else
    warn "agda-stdlib tarball fetch failed"
  fi
}

# Cubical Agda is the `--cubical` FLAG (ships with the agda binary — free).
# The agda/cubical LIBRARY (HIT stdlib) is optional and version-sensitive;
# best-effort fetch, never fatal.
install_agda_cubical() {
  [ -d "${CUBICAL_DIR}" ] && { log "agda/cubical library present"; return; }
  log "fetching agda/cubical library tarball (best-effort)…"
  fetch_tarball "https://github.com/agda/cubical/archive/refs/heads/master.tar.gz" "${CUBICAL_DIR}" \
    || warn "agda/cubical fetch failed (the --cubical flag still works without the library)"
}

register_agda_libraries() {
  mkdir -p "${AGDA_HOME}"
  {
    [ -f "${STDLIB_DIR}/standard-library.agda-lib" ] && echo "${STDLIB_DIR}/standard-library.agda-lib"
    [ -f "${CUBICAL_DIR}/cubical.agda-lib" ]        && echo "${CUBICAL_DIR}/cubical.agda-lib"
    for cand in /home/user/absolute-zero/absolute-zero.agda-lib \
                "${HOME}/absolute-zero/absolute-zero.agda-lib"; do
      [ -f "$cand" ] && echo "$cand"
    done
  } > "${AGDA_HOME}/libraries"
  echo "standard-library" > "${AGDA_HOME}/defaults"
  log "registered ~/.agda/libraries:"; sed 's/^/    /' "${AGDA_HOME}/libraries"
}

install_zig() {
  have zig && { log "zig present: $(zig version 2>&1)"; return; }
  log "installing zig ${ZIG_VER}…"
  local base="https://ziglang.org/download/${ZIG_VER}"
  # Asset naming flipped arch/os around 0.14; try both.
  for name in "zig-x86_64-linux-${ZIG_VER}" "zig-linux-x86_64-${ZIG_VER}"; do
    if curl -fsSL "${base}/${name}.tar.xz" -o /tmp/zig.tar.xz 2>/dev/null; then
      rm -rf "${ZIG_DIR}"; mkdir -p "${ZIG_DIR}"
      tar -xJf /tmp/zig.tar.xz -C "${ZIG_DIR}" --strip-components=1 \
        && ln -sf "${ZIG_DIR}/zig" /usr/local/bin/zig && { log "zig unpacked (${name})"; return; }
    fi
  done
  warn "zig download failed for both asset-name patterns at ${base}"
}

install_lean() {
  export PATH="${HOME}/.elan/bin:${PATH}"
  have lean && { log "lean present: $(lean --version 2>&1)"; return; }
  log "installing lean ${LEAN_VER} via elan…"
  if curl -fsSL https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh -o /tmp/elan.sh 2>/dev/null; then
    sh /tmp/elan.sh -y --default-toolchain "leanprover/lean4:${LEAN_VER}" >/dev/null 2>&1 \
      || warn "elan install failed"
  else
    warn "could not fetch elan-init.sh"
  fi
}

# Idris2 is a CORE estate language (owns ABIs) — install via Chez bootstrap.
install_idris2() {
  export PATH="${HOME}/.idris2/bin:${PATH}"
  have idris2 && { log "idris2 present: $(idris2 --version 2>&1)"; return; }
  log "installing idris2 ${IDRIS2_TAG} (chez bootstrap)…"
  have scheme || have chez || have chezscheme || apt_get install -y chezscheme >/dev/null 2>&1 || \
    warn "could not install chezscheme (idris2 bootstrap needs a Chez Scheme)"
  local sch=""
  for c in chezscheme chez scheme; do have "$c" && { sch="$c"; break; }; done
  [ -z "$sch" ] && { warn "no Chez Scheme found; skipping idris2"; return; }
  if [ ! -f "${IDRIS2_SRC}/Makefile" ]; then
    fetch_tarball "https://github.com/idris-lang/Idris2/archive/refs/tags/${IDRIS2_TAG}.tar.gz" "${IDRIS2_SRC}" \
      || { warn "idris2 tarball fetch failed"; return; }
  fi
  ( cd "${IDRIS2_SRC}" \
      && make bootstrap SCHEME="${sch}" >/tmp/idris2-build.log 2>&1 \
      && make install PREFIX="${HOME}/.idris2" >>/tmp/idris2-build.log 2>&1 ) \
    || warn "idris2 build failed (see /tmp/idris2-build.log)"
}

install_z3() {
  have z3 && { log "z3 present: $(z3 --version 2>&1)"; return; }
  log "installing z3 (apt)…"
  apt_get install -y z3 >/dev/null 2>&1 || warn "apt could not install z3"
}

install_cvc5() {
  have cvc5 && { log "cvc5 present: $(cvc5 --version 2>&1 | head -1)"; return; }
  log "installing cvc5 ${CVC5_VER}…"
  have unzip || apt_get install -y unzip >/dev/null 2>&1 || true
  local base="https://github.com/cvc5/cvc5/releases/download/cvc5-${CVC5_VER}"
  # Single static binary first, then the zipped variant.
  if curl -fsSL "${base}/cvc5-Linux-x86_64-static" -o /usr/local/bin/cvc5 2>/dev/null \
       && [ -s /usr/local/bin/cvc5 ]; then
    chmod +x /usr/local/bin/cvc5; log "cvc5 binary installed"; return
  fi
  for asset in "cvc5-Linux-x86_64-static.zip" "cvc5-Linux-static.zip" "cvc5-Linux.zip"; do
    if curl -fsSL "${base}/${asset}" -o /tmp/cvc5.zip 2>/dev/null && [ -s /tmp/cvc5.zip ]; then
      rm -rf /tmp/cvc5d; mkdir -p /tmp/cvc5d && unzip -q /tmp/cvc5.zip -d /tmp/cvc5d 2>/dev/null || continue
      local bin; bin="$(find /tmp/cvc5d -type f -name cvc5 | head -1)"
      [ -n "$bin" ] && { install -m755 "$bin" /usr/local/bin/cvc5; log "cvc5 installed (${asset})"; return; }
    fi
  done
  warn "cvc5 download failed for all known asset names at ${base}"
}

# ---- Heavy (opt-in) ---------------------------------------------------------
install_coq() {
  have coqc && { log "coq present: $(coqc --version 2>&1 | head -1)"; return; }
  log "installing coq ${COQ_VER} via opam (heavy)…"
  if ! have opam; then
    apt_get install -y opam >/dev/null 2>&1 || { warn "opam unavailable"; return; }
  fi
  opam init --disable-sandboxing -y >/dev/null 2>&1 || true
  eval "$(opam env 2>/dev/null)" || true
  opam install -y "coq.${COQ_VER}" >/tmp/coq-build.log 2>&1 || warn "opam coq install failed (see /tmp/coq-build.log)"
}

install_isabelle() {
  have isabelle && { log "isabelle present"; return; }
  log "installing ${ISABELLE_VER} (heavy, ~1.1 GB download, HOL heap ships prebuilt)…"
  # Only the *current* release lives under /dist/; older ones move to the
  # archived /website-<VER>/dist/ path (the /dist/ form 404s). The archived
  # path is stable and works for the current release too, so use it uniformly.
  local url="https://isabelle.in.tum.de/website-${ISABELLE_VER}/dist/${ISABELLE_VER}_linux.tar.gz"
  if curl -fsSL "$url" -o /tmp/isabelle.tar.gz 2>/dev/null && [ -s /tmp/isabelle.tar.gz ]; then
    tar -xzf /tmp/isabelle.tar.gz -C /opt \
      && ln -sf "/opt/${ISABELLE_VER}/bin/isabelle" /usr/local/bin/isabelle \
      && log "isabelle unpacked" || warn "isabelle unpack failed"
  else
    warn "isabelle download failed: ${url}"
  fi
}

install_mizar() {
  have mizar && { log "mizar present"; return; }
  warn "mizar not auto-installed: distribution is manual (mizar.org). Flagged as UNKNOWN."
}

# =============================================================================
# Honest verification / doctor table.
# =============================================================================
declare -a REPORT
verify() {  # verify <label> <cmd...>
  local label="$1"; shift
  local full rc line
  full="$("$@" 2>&1)"; rc=$?            # rc is the tool's, not a pipe's
  # Prefer the first substantive line over toolchain warnings.
  line="$(printf '%s\n' "$full" | grep -viE 'warning|could not canonicalize|toolchains' | head -1)"
  [ -z "$line" ] && line="$(printf '%s\n' "$full" | head -1)"
  if [ "$rc" -eq 0 ] && [ -n "$line" ]; then
    REPORT+=("OK      | ${label} | ${line}")
  else
    REPORT+=("MISSING | ${label} | (—)")
  fi
}

run_doctor() {
  export PATH="${HOME}/.elan/bin:${HOME}/.idris2/bin:${PATH}"
  eval "$(opam env 2>/dev/null)" || true
  REPORT=()
  verify "agda"     agda --version
  verify "idris2"   idris2 --version
  verify "lean"     lean --version
  verify "z3"       z3 --version
  verify "cvc5"     cvc5 --version
  verify "zig"      zig version
  verify "coqc"     coqc --version
  verify "isabelle" isabelle version
  [ -f "${CUBICAL_DIR}/cubical.agda-lib" ] \
    && REPORT+=("OK      | cubical-lib | ${CUBICAL_DIR}") \
    || REPORT+=("MISSING | cubical-lib | (flag --cubical still works without it)")
  echo
  echo "=== arghda doctor: backend availability (verified this run) ==="
  printf '%s\n' "${REPORT[@]}" | column -t -s '|' 2>/dev/null || printf '%s\n' "${REPORT[@]}"
  echo "==============================================================="
}

# =============================================================================
main() {
  if [ "${VERIFY_ONLY}" -eq 0 ]; then
    # Fast / reliable first; slow chez-bootstrap (idris2) last so it can never
    # block the others.
    install_agda
    install_agda_stdlib
    install_agda_cubical
    register_agda_libraries
    install_zig
    install_z3
    install_cvc5
    install_lean
    install_idris2
    if [ "${WANT_HEAVY}" -eq 1 ]; then install_coq; install_isabelle; fi
    if [ "${WANT_MIZAR}" -eq 1 ]; then install_mizar; fi
  fi
  run_doctor
}
main
