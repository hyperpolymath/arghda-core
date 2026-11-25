// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The *code view* of an Idris2 source file: comments and the contents of
//! string and character literals become spaces, newlines are kept, and
//! everything else is copied. The Idris2 lint rules scan this view, so text
//! that the compiler does not read as code cannot raise or hide a finding.
//!
//! The rules follow the Idris2 0.7.0 lexer (`src/Parser/Lexer/Source.idr`
//! and `Common.idr` at tag `v0.7.0`):
//!
//! * a line comment is `--`, then any further dashes, *not* followed by `}`,
//!   to the end of the line; `|||` starts a documentation comment, also to
//!   the end of the line;
//! * a block comment is `{-` … `-}` and nests. Inside it the lexer skips
//!   string literals, character literals and `--` line comments when it
//!   matches delimiters, so a `{-` or `-}` inside them does not count;
//! * a string starts with `n` hashes and `"` (or `"""` followed only by
//!   spaces to the end of the line, for a multi-line string) and ends at `"`
//!   (or `"""`) followed by the same `n` hashes. An escape is `\` followed by
//!   `n` hashes and one character. A single-line string may still contain a
//!   newline.
//!
//! Fail-closed. Wherever this view cannot be sure of the lexer's reading, it
//! stops blanking and copies the rest of the file unchanged, so a lint can
//! over-report but never miss code. That happens at an unclosed comment or
//! string, at a string interpolation `\{ … }` (which holds code), and at a
//! `"""` opener that has more text on its line.

/// The code view of `src` (see the module docs).
pub fn code_view(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        let rest = &b[i..];
        if rest.starts_with(b"|||") {
            let end = line_end(b, i);
            blank_into(&mut out, &src[i..end]);
            i = end;
        } else if rest.starts_with(b"--") {
            let mut j = i + 2;
            while j < b.len() && b[j] == b'-' {
                j += 1;
            }
            if j < b.len() && b[j] == b'}' {
                // Dashes then `}` is a stray closing delimiter, not a
                // comment; copy it.
                out.push_str(&src[i..=j]);
                i = j + 1;
            } else {
                let end = line_end(b, i);
                blank_into(&mut out, &src[i..end]);
                i = end;
            }
        } else if rest.starts_with(b"{-") {
            match block_end(b, i) {
                Some(end) => {
                    blank_into(&mut out, &src[i..end]);
                    i = end;
                }
                None => return keep_rest(out, src, i),
            }
        } else if rest[0] == b'"' || (rest[0] == b'#' && hashes_then_quote(b, i).is_some()) {
            let hashes = if rest[0] == b'#' {
                hashes_then_quote(b, i).expect("checked")
            } else {
                0
            };
            match string_end(b, i, hashes) {
                StringScan::Closed { open, close, end } => {
                    // Keep the delimiters, blank the contents.
                    out.push_str(&src[i..open]);
                    blank_into(&mut out, &src[open..close]);
                    out.push_str(&src[close..end]);
                    i = end;
                }
                StringScan::Uncertain => return keep_rest(out, src, i),
            }
        } else if rest[0] == b'\'' && !follows_ident(b, i) {
            match char_lit_end(b, i) {
                Some(end) => {
                    out.push('\'');
                    blank_into(&mut out, &src[i + 1..end - 1]);
                    out.push('\'');
                    i = end;
                }
                None => {
                    out.push('\'');
                    i += 1;
                }
            }
        } else {
            let ch = src[i..].chars().next().expect("in bounds");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Append `s` with every character except newlines replaced by a space.
fn blank_into(out: &mut String, s: &str) {
    for ch in s.chars() {
        out.push(if ch == '\n' { '\n' } else { ' ' });
    }
}

/// Stop blanking: copy the rest of the file unchanged (fail-closed).
fn keep_rest(mut out: String, src: &str, i: usize) -> String {
    out.push_str(&src[i..]);
    out
}

/// Index of the `\n` ending the line that holds `i`, or the end of input.
fn line_end(b: &[u8], i: usize) -> usize {
    b[i..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |p| i + p)
}

/// `true` when the `'` at `i` continues an identifier (`x'`), where Idris2
/// reads it as part of the name, not as a character literal.
fn follows_ident(b: &[u8], i: usize) -> bool {
    i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'\'')
}

/// The number of `#` before a `"` starting at `i`, if `i` starts `#…#"`.
fn hashes_then_quote(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    while j < b.len() && b[j] == b'#' {
        j += 1;
    }
    (j < b.len() && b[j] == b'"').then_some(j - i)
}

/// A character literal `'c'` or `'\c'` starting at `i`: the index just
/// after its closing quote.
fn char_lit_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if j >= b.len() {
        return None;
    }
    if b[j] == b'\\' {
        j += 1;
        if j >= b.len() {
            return None;
        }
    } else if b[j] == b'\'' {
        return None;
    }
    // One (possibly multi-byte) character.
    j += utf8_len(b[j]);
    (j < b.len() && b[j] == b'\'').then_some(j + 1)
}

fn utf8_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

enum StringScan {
    /// Contents are `open..close`; the literal ends at `end`.
    Closed {
        open: usize,
        close: usize,
        end: usize,
    },
    /// Unclosed, interpolated, or a `"""` this view does not follow.
    Uncertain,
}

/// Scan a string literal whose `hashes` hashes start at `i`.
fn string_end(b: &[u8], i: usize, hashes: usize) -> StringScan {
    let q = i + hashes; // the first `"`
    let multi = b[q..].starts_with(b"\"\"\"");
    if multi {
        // `"""` must be followed only by spaces to the end of the line.
        let mut j = q + 3;
        while j < b.len() && b[j] != b'\n' {
            if !b[j].is_ascii_whitespace() {
                return StringScan::Uncertain;
            }
            j += 1;
        }
    }
    let open = if multi { q + 3 } else { q + 1 };
    let delim: &[u8] = if multi { b"\"\"\"" } else { b"\"" };
    let mut j = open;
    while j < b.len() {
        if b[j] == b'\\' && run_of(b, j + 1, b'#') >= hashes {
            let k = j + 1 + hashes;
            if k < b.len() && b[k] == b'{' {
                return StringScan::Uncertain; // interpolation holds code
            }
            if k >= b.len() {
                return StringScan::Uncertain;
            }
            j = k + utf8_len(b[k]);
            continue;
        }
        if b[j..].starts_with(delim) && run_of(b, j + delim.len(), b'#') >= hashes {
            let end = j + delim.len() + hashes;
            if !multi && hashes == 0 && end < b.len() && b[end] == b'"' {
                // The lexer refuses a closing quote followed by another one.
                return StringScan::Uncertain;
            }
            return StringScan::Closed {
                open,
                close: j,
                end,
            };
        }
        j += 1;
    }
    StringScan::Uncertain
}

fn run_of(b: &[u8], i: usize, c: u8) -> usize {
    b[i.min(b.len())..].iter().take_while(|&&x| x == c).count()
}

/// End of the block comment opened at `start` (`{-`), following the
/// Idris2 0.7.0 comment automaton; `None` if it is not closed.
fn block_end(b: &[u8], start: usize) -> Option<usize> {
    // `blockComment = {- many(-) (eof | toEndComment 1)`.
    let mut i = start + 2;
    while i < b.len() && b[i] == b'-' {
        i += 1;
    }
    let mut depth = 1usize;
    loop {
        // toEndComment
        let c = *b.get(i)?;
        match c {
            b'{' => {
                i += 1;
                if b.get(i) == Some(&b'-') {
                    // singleBrace: an opening delimiter, its dashes, then
                    // singleDash at the new depth (`{--}` closes at once).
                    i += 1;
                    while i < b.len() && b[i] == b'-' {
                        i += 1;
                    }
                    depth += 1;
                    if b.get(i) == Some(&b'}') {
                        i += 1;
                        depth -= 1;
                    }
                }
            }
            b'-' => {
                i += 1;
                match b.get(i) {
                    Some(b'-') => {
                        // doubleDash: absorb dashes; `}` closes, else a line
                        // comment to the end of the line.
                        while i < b.len() && b[i] == b'-' {
                            i += 1;
                        }
                        if b.get(i) == Some(&b'}') {
                            i += 1;
                            depth -= 1;
                            if depth == 0 {
                                return Some(i);
                            }
                        } else {
                            i = line_end(b, i);
                        }
                    }
                    Some(b'}') => {
                        i += 1;
                        depth -= 1;
                        if depth == 0 {
                            return Some(i);
                        }
                    }
                    _ => {}
                }
            }
            b'\'' => {
                i = char_lit_end(b, i).unwrap_or(i + 1);
            }
            b'"' => {
                // stringLit: `"` (escape `\` any | any)* `"`, newlines allowed.
                i += 1;
                loop {
                    match *b.get(i)? {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            _ => i += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::code_view;

    fn same_lines(src: &str) {
        assert_eq!(code_view(src).lines().count(), src.lines().count());
    }

    #[test]
    fn comments_become_spaces_and_lines_are_kept() {
        let src = "x = 1 -- believe_me\n||| doc ?h\n{- a\n ?b -}\ny = ?c\n";
        let v = code_view(src);
        same_lines(src);
        assert!(!v.contains("believe_me") && !v.contains("?h") && !v.contains("?b"));
        assert!(v.contains("?c"));
    }

    #[test]
    fn string_and_char_contents_are_blanked() {
        let v = code_view("s = \"?x --\" ++ show 'a'\ny = ?h\n");
        assert!(!v.contains("?x") && !v.contains("--"));
        assert!(v.contains("?h"));
    }

    #[test]
    fn codex_p1_dash_dash_in_a_string_hides_no_hole() {
        let v = code_view("g = let s = \" --\" in ?todo\n");
        assert!(v.contains("?todo"), "{v:?}");
    }

    #[test]
    fn doc_comment_opener_does_not_open_a_block() {
        let v = code_view("module M\n||| Example opener: {-\nx : Nat\nx = ?todo\n-- -}\n");
        assert!(v.contains("?todo"), "{v:?}");
    }

    #[test]
    fn triple_quoted_string_hides_its_opener() {
        let v = code_view("note = \"\"\"\n  {-\n  \"\"\"\nx = ?todo\n-- -}\n");
        assert!(v.contains("?todo"), "{v:?}");
    }

    #[test]
    fn raw_string_closes_only_on_matching_hashes() {
        let v = code_view("note = #\"a \"{-\"#\nx = ?todo\n-- -}\n");
        assert!(v.contains("?todo"), "{v:?}");
    }

    #[test]
    fn quoted_opener_inside_a_comment_does_not_nest() {
        let v = code_view("{- example \"{-\" -}\nx = ?todo\n-- -}\n");
        assert!(v.contains("?todo"), "{v:?}");
    }

    #[test]
    fn line_comment_inside_a_block_hides_a_close() {
        // `-- -}` inside a block comment is a line comment, so the block
        // closes only at the later `-}`.
        let v = code_view("{- a -- -}\n ?hidden -}\ny = ?shown\n");
        assert!(!v.contains("?hidden"), "{v:?}");
        assert!(v.contains("?shown"));
    }

    #[test]
    fn nested_and_empty_comments() {
        let v = code_view("{- a {- b -} ?in -} {--} y = ?out\n");
        assert!(!v.contains("?in"));
        assert!(v.contains("?out"));
    }

    #[test]
    fn uncertain_input_is_kept_verbatim() {
        for src in [
            "{- never closed\nx = ?todo\n",
            "s = \"never closed\nx = ?todo\n",
            "s = \"\\{ ?inside }\"\nx = ?todo\n",
            "s = \"\"\" text\nx = ?todo\n",
        ] {
            assert!(code_view(src).contains("?todo"), "{src:?}");
        }
    }

    #[test]
    fn primed_identifier_is_not_a_char_literal() {
        let v = code_view("x' = y' ++ ?h\n");
        assert!(v.contains("x'") && v.contains("?h"));
    }

    #[test]
    fn dashes_before_a_brace_are_not_a_line_comment() {
        // `--}` outside a comment is a stray delimiter; what follows is code.
        let v = code_view("x --} ?h\n");
        assert!(v.contains("?h"));
    }

    #[test]
    fn multibyte_text_survives() {
        assert_eq!(code_view("λ {- ∀ -} →"), "λ         →");
    }
}
