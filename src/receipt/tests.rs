// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Known-answer tests for the receipt reader and the derivation.

use super::*;

/// The worked example of standards PROOF-POLICY §8.5, copied verbatim from
/// `metadatastician/standards` `1-formats/deed/tools/fixtures/proof-receipt/
/// example_praxis.deed` at 2c7467bde91bf35d6a26d1838154db50680d3cdb.
const STANDARDS_EXAMPLE: &str =
    include_str!("../../tests/fixtures/receipts/standards-example_praxis.deed");

const SHA_A: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
const SHA_B: &str = "a1b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn files() -> Vec<(String, String)> {
    vec![
        ("arghda.agda-lib".to_string(), SHA_A.to_string()),
        ("src/M.agda".to_string(), SHA_B.to_string()),
    ]
}

fn digest() -> String {
    manifest_digest(&files())
}

/// A workspace receipt for module `M`, agda leg, with the given extra
/// receipt text (`:verdict …`, `(lint …)`) and state.
fn deed(extra: &str, state: &str) -> String {
    deed_with(&files(), extra, state)
}

/// As [`deed`], over the given manifest, listed in the given order, whose
/// `:inputs-sha256` is always the digest of that listing. So a refusal of a
/// bad listing cannot come from a digest mismatch instead.
fn deed_with(files: &[(String, String)], extra: &str, state: &str) -> String {
    let reason = if state == "current" {
        String::new()
    } else {
        ":reason \"test\"".to_string()
    };
    let manifest: String = files
        .iter()
        .map(|(p, s)| format!("(file :path \"{p}\" :sha256 \"{s}\")"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "(praxis-deed :schema-version \"1.0.0\" :profile workspace-receipt
           (claim :id \"M\" (theorem :name \"m-thm\")
             (leg :prover agda
               (receipt :checker \"agda 2.6.3\" :invocation \"agda --safe --without-K\"
                        :inputs-sha256 \"{d}\" :state {state} {reason} {extra}
                        (inputs-manifest {manifest})))))",
        d = manifest_digest(files)
    )
}

fn derive(extra: &str) -> Derivation {
    ReceiptDeed::parse(&deed(extra, "current"))
        .unwrap()
        .derive("M", "agda", &digest())
}

const CLEAN: &str = "(lint :pack \"arghda-0.1\")";

#[test]
fn standards_example_parses_and_its_digest_is_recomputed() {
    let d = ReceiptDeed::parse(STANDARDS_EXAMPLE).unwrap();
    assert_eq!(d.profile, Profile::ProofReceipt);
    assert_eq!(d.source_commit.as_deref(), Some(COMMIT));
    let leg = &d.claims[0].legs[0];
    let cur = leg
        .receipts
        .iter()
        .find(|r| r.state == ReceiptState::Current)
        .unwrap();
    // The reader rebuilt the manifest and it matched (parse would have
    // failed otherwise); check the value against the standards fixture too.
    assert_eq!(
        manifest_digest(&cur.manifest),
        "c8945990a8e4242d7eaeaa42dd4e647a4e372fdbd32714fb534744df85066248"
    );
}

#[test]
fn a_standards_receipt_today_is_not_evidence_of_a_verdict() {
    // Gap 3: the receipt records no verdict, so it cannot derive `proven`.
    let d = ReceiptDeed::parse(STANDARDS_EXAMPLE).unwrap();
    let got = d.derive(
        "T-1",
        "agda",
        "c8945990a8e4242d7eaeaa42dd4e647a4e372fdbd32714fb534744df85066248",
    );
    assert_eq!(got, Derivation::unknown(Why::NoVerdict));
}

#[test]
fn a_complete_matching_receipt_derives_proven() {
    let got = derive(&format!(":verdict proven {CLEAN}"));
    assert_eq!(got.verdict, Verdict::Proven);
    assert_eq!(got.why, Why::Recorded);
}

#[test]
fn theorems_are_listed_not_graded() {
    let d = ReceiptDeed::parse(&deed(&format!(":verdict proven {CLEAN}"), "current")).unwrap();
    assert_eq!(d.claims[0].theorems, vec!["m-thm".to_string()]);
}

#[test]
fn a_changed_input_makes_the_receipt_stale() {
    let d = ReceiptDeed::parse(&deed(&format!(":verdict proven {CLEAN}"), "current")).unwrap();
    let other = "0".repeat(64);
    let got = d.derive("M", "agda", &other);
    assert_eq!(got.verdict, Verdict::Unknown);
    assert!(matches!(got.why, Why::Stale { .. }));
}

#[test]
fn lint_clean_but_rejected_by_the_prover_is_not_proven() {
    // The reproduction in the design note: lint-clean, but the prover
    // rejects it. Location used to make this "proven"; a receipt cannot.
    let got = derive(&format!(":verdict error {CLEAN}"));
    assert_eq!(got.verdict, Verdict::Error);
}

#[test]
fn a_hard_block_is_never_better_than_unknown() {
    let got = derive(
        ":verdict proven (lint :pack \"arghda-0.1\" \
         (finding :rule \"unjustified-postulate\" :severity hard-block))",
    );
    assert_eq!(got.verdict, Verdict::Unknown);
    assert_eq!(
        got.why,
        Why::HardBlock(vec!["unjustified-postulate".into()])
    );
    // A hard block on a rule with no cap (orphan-module) is still blocked.
    let got = derive(
        ":verdict proven (lint :pack \"arghda-0.1\" \
         (finding :rule \"orphan-module\" :severity hard-block))",
    );
    assert_eq!(got.verdict, Verdict::Unknown);
}

#[test]
fn a_recorded_warning_still_caps_the_verdict() {
    // Defence in depth: even if a writer recorded `proven` next to an
    // escape hatch, the reader applies the same cap as `check`.
    let got = derive(
        ":verdict proven (lint :pack \"arghda-0.1\" \
         (finding :rule \"escape-hatch\" :severity warn))",
    );
    assert_eq!(got.verdict, Verdict::Admitted);
}

#[test]
fn no_lint_evidence_is_unknown() {
    assert_eq!(
        derive(":verdict proven"),
        Derivation::unknown(Why::NoLintEvidence)
    );
}

#[test]
fn no_receipt_at_all_is_unknown() {
    // A `proven/` file with no receipt (the migration ruling).
    let d = ReceiptDeed::parse(&deed(&format!(":verdict proven {CLEAN}"), "current")).unwrap();
    assert_eq!(
        d.derive("Other", "agda", &digest()),
        Derivation::unknown(Why::NoClaim)
    );
    assert_eq!(
        d.derive("M", "lean4", &digest()),
        Derivation::unknown(Why::NoLeg)
    );
}

#[test]
fn only_a_current_receipt_counts() {
    let d = ReceiptDeed::parse(&deed(&format!(":verdict proven {CLEAN}"), "superseded")).unwrap();
    assert_eq!(
        d.derive("M", "agda", &digest()),
        Derivation::unknown(Why::NoCurrentReceipt)
    );
}

#[test]
fn malformed_receipts_are_refused() {
    let extra = format!(":verdict proven {CLEAN}");
    let ok = deed(&extra, "current");
    let listing = |fs: &[(&str, &str)]| -> String {
        let v: Vec<(String, String)> = fs
            .iter()
            .map(|(p, s)| (p.to_string(), s.to_string()))
            .collect();
        deed_with(&v, &extra, "current")
    };
    // (what, deed, a fragment the error must contain)
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "digest not of its manifest",
            ok.replace(&digest(), &"1".repeat(64)),
            "is not the SHA-256",
        ),
        (
            "manifest out of order",
            listing(&[("zz.agda-lib", SHA_A), ("src/M.agda", SHA_B)]),
            "bytewise order",
        ),
        (
            "path listed twice",
            listing(&[("src/M.agda", SHA_A), ("src/M.agda", SHA_B)]),
            "bytewise order",
        ),
        ("path with ..", listing(&[("../M.agda", SHA_A)]), "`..`"),
        ("leading /", listing(&[("/M.agda", SHA_A)]), "empty"),
        (
            "short sha",
            listing(&[("M.agda", "abc")]),
            "64 lowercase hex",
        ),
        ("empty manifest", listing(&[]), "lists no file"),
        (
            "unknown field",
            ok.replace(":verdict proven", ":verdict proven :colour blue"),
            "unknown field :colour",
        ),
        (
            "unknown verdict",
            ok.replace(":verdict proven", ":verdict green"),
            "unknown :verdict",
        ),
        (
            "unknown severity",
            ok.replace(
                "(lint :pack \"arghda-0.1\")",
                "(lint :pack \"p\" (finding :rule \"r\" :severity loud))",
            ),
            "unknown :severity",
        ),
        (
            "two current receipts",
            {
                // The same valid current receipt, twice in one leg.
                let start = ok.find("(receipt").unwrap();
                let end = ok.len() - ")))".len();
                let r = &ok[start..end];
                format!("{}{r} {r})))", &ok[..start])
            },
            "more than one current receipt",
        ),
        (
            "not current without reason",
            ok.replace(":state current", ":state stale"),
            "needs :reason",
        ),
        (
            "proof-receipt without a repository",
            ok.replace(":profile workspace-receipt", ":profile proof-receipt"),
            "needs :repository",
        ),
        (
            "proof-receipt without a source commit",
            STANDARDS_EXAMPLE.replace(
                ":source-commit \"0123456789abcdef0123456789abcdef01234567\"",
                "",
            ),
            "40-character :source-commit",
        ),
        (
            "no schema version",
            ok.replace(":schema-version \"1.0.0\"", ""),
            "needs :schema-version",
        ),
        (
            "unknown schema version",
            ok.replace(":schema-version \"1.0.0\"", ":schema-version \"2.0.0\""),
            "unsupported :schema-version",
        ),
        (
            "non-string schema version",
            ok.replace(":schema-version \"1.0.0\"", ":schema-version #t"),
            "must be a string",
        ),
        (
            "blank lint pack",
            ok.replace("(lint :pack \"arghda-0.1\")", "(lint :pack \"  \")"),
            "non-blank :pack",
        ),
        (
            "checker names another prover",
            ok.replace(":checker \"agda 2.6.3\"", ":checker \"z3 4.13\""),
            "must name the agda leg's prover",
        ),
        (
            "checker without a version",
            ok.replace(":checker \"agda 2.6.3\"", ":checker \"agda\""),
            "and a version",
        ),
        (
            "raw carriage return in a deed string",
            listing(&[("M\r.agda", SHA_A)]),
            "raw control character",
        ),
    ];
    for (what, src, expect) in cases {
        let err = match ReceiptDeed::parse(&src) {
            Ok(_) => panic!("accepted: {what}"),
            Err(e) => format!("{e:#}"),
        };
        assert!(
            err.contains(expect),
            "{what}: refused for another reason: {err}"
        );
    }
}

/// An `(import …)` clause for `import_tests`, with the given files and the
/// digest that a correct clause carries unless `digest` overrides it.
fn with_import(repo: &str, files: &[(String, String)], digest: Option<&str>) -> String {
    let d = digest.map_or_else(|| manifest_digest(files), str::to_string);
    let fs: String = files
        .iter()
        .map(|(p, s)| format!("(file :path \"{p}\" :sha256 \"{s}\")"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("(import :repository \"{repo}\" :commit \"{COMMIT}\" :inputs-sha256 \"{d}\" {fs})")
}

#[test]
fn imports_are_checked_like_the_home_manifest() {
    let extra = format!(":verdict proven {CLEAN}");
    let base = deed(&extra, "current");
    let one = vec![("lib/L.agda".to_string(), SHA_A.to_string())];
    let add =
        |clause: String| base.replace("(inputs-manifest", &format!("{clause} (inputs-manifest"));
    // A correct import is accepted.
    assert!(ReceiptDeed::parse(&add(with_import("o/lib", &one, None))).is_ok());
    // Refusals, each for its own reason.
    let bad = [
        (
            add(with_import("o/lib", &one, Some(&"0".repeat(64)))),
            "is not the SHA-256 of its files",
        ),
        (add(with_import("not-a-repo", &one, None)), "OWNER/REPO"),
        (
            add(format!(
                "{} {}",
                with_import("o/lib", &one, None),
                with_import("O/Lib", &one, None)
            )),
            "listed twice",
        ),
        (add(with_import("o/lib", &[], None)), "lists no file"),
    ];
    for (src, expect) in bad {
        let err = format!("{:#}", ReceiptDeed::parse(&src).unwrap_err());
        assert!(err.contains(expect), "{expect}: {err}");
    }
}

#[test]
fn backend_names_map_to_deed_provers() {
    assert_eq!(deed_prover_for_backend("agda"), "agda");
    assert_eq!(deed_prover_for_backend("agda-cubical"), "agda");
    assert_eq!(deed_prover_for_backend("coq"), "rocq");
    assert_eq!(deed_prover_for_backend("lean4"), "lean");
    assert_eq!(deed_prover_for_backend("idris2"), "idris2");
    // The agda leg is found from the cubical backend's name.
    let d = ReceiptDeed::parse(&deed(&format!(":verdict proven {CLEAN}"), "current")).unwrap();
    assert_eq!(
        d.derive_for_backend("M", "agda-cubical", &digest()).verdict,
        Verdict::Proven
    );
}

#[cfg(unix)]
#[test]
fn worktree_manifest_refuses_symbolic_links() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"x").unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/M.agda"), b"m").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("link.agda"))
        .unwrap();
    std::os::unix::fs::symlink(dir.path().join("src"), dir.path().join("alias")).unwrap();
    for p in ["link.agda", "alias/M.agda"] {
        let e = worktree_manifest(dir.path(), &[p.to_string()]).unwrap_err();
        assert!(e.to_string().contains("symbolic link"), "{p}: {e}");
    }
    assert!(worktree_manifest(dir.path(), &["src/M.agda".to_string()]).is_ok());
}

#[test]
fn worktree_manifest_sorts_bytewise_and_digests() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/M.agda"), b"module M where\n").unwrap();
    std::fs::write(dir.path().join("B.agda-lib"), b"name: b\n").unwrap();
    let (files, d) = worktree_manifest(
        dir.path(),
        &["src/M.agda".to_string(), "B.agda-lib".to_string()],
    )
    .unwrap();
    assert_eq!(files[0].0, "B.agda-lib");
    assert_eq!(files[1].0, "src/M.agda");
    assert_eq!(files[0].1, sha256_hex(b"name: b\n"));
    assert_eq!(d, manifest_digest(&files));
    assert!(worktree_manifest(dir.path(), &["../x".to_string()]).is_err());
    // A carriage return can reach the path rule only through this API (a
    // deed string cannot hold one); `sha256sum` escapes it, so it is refused.
    std::fs::write(dir.path().join("C\r.agda"), b"c").unwrap();
    let e = worktree_manifest(dir.path(), &["C\r.agda".to_string()]).unwrap_err();
    assert!(
        e.to_string().contains("control character or a backslash"),
        "{e}"
    );
    assert!(worktree_manifest(dir.path(), &[]).is_err());
}

/// The receipt rule, checked over every combination the fixtures can
/// express: `Proven` is derived only when the receipt is current, matches
/// the input digest, records `proven`, has lint evidence, and has no hard
/// block or capping warning.
#[test]
fn derived_proven_implies_the_receipt_rule() {
    let verdicts = [
        "proven",
        "admitted",
        "postulated",
        "unknown",
        "refuted",
        "error",
        "unavailable",
    ];
    let lints = [
        None,
        Some(CLEAN.to_string()),
        Some("(lint :pack \"p\" (finding :rule \"escape-hatch\" :severity warn))".to_string()),
        Some("(lint :pack \"p\" (finding :rule \"tab-mix\" :severity warn))".to_string()),
        Some(
            "(lint :pack \"p\" (finding :rule \"orphan-module\" :severity hard-block))".to_string(),
        ),
    ];
    let mut proven = 0;
    let mut total = 0;
    for state in ["current", "stale", "superseded", "regressed"] {
        for v in [None, Some(verdicts[0])]
            .into_iter()
            .chain(verdicts[1..].iter().map(|v| Some(*v)))
        {
            for lint in &lints {
                for matching in [true, false] {
                    let extra = format!(
                        "{} {}",
                        v.map(|v| format!(":verdict {v}")).unwrap_or_default(),
                        lint.clone().unwrap_or_default()
                    );
                    let d = ReceiptDeed::parse(&deed(&extra, state)).unwrap();
                    let key = if matching { digest() } else { "f".repeat(64) };
                    let got = d.derive("M", "agda", &key);
                    total += 1;
                    if got.verdict == Verdict::Proven {
                        proven += 1;
                        assert_eq!(state, "current");
                        assert!(matching);
                        assert_eq!(v, Some("proven"));
                        let l = lint.as_deref().expect("lint evidence");
                        assert!(!l.contains("hard-block") && !l.contains("escape-hatch"));
                    }
                }
            }
        }
    }
    // 4 states x 8 verdict options x 5 lint options x 2 digests.
    assert_eq!(total, 320);
    // Exactly the two clean lint packs, current, matching, `proven`.
    assert_eq!(proven, 2);
}
