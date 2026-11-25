// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! A small, fail-closed reader for the deed s-expression syntax.
//!
//! It reads exactly what a proof-receipt deed needs: lists, double-quoted
//! strings (with `\"` and `\\` escapes), symbols, `:keywords`, the booleans
//! `#t`/`#f`, tagged literals such as `#u8"…"` or `#uuid"…"`, and `;` line
//! comments. Anything else is an error, never a guess. It is not the deed
//! grammar's validator: the normative grammar is standards
//! `1-formats/deed/spec/abnf/deed.abnf`, and `deed_lint.js` checks it.

use anyhow::{bail, Result};

/// One s-expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sx {
    /// `( … )`
    List(Vec<Sx>),
    /// `"…"`, with escapes resolved.
    Str(String),
    /// A bare symbol, such as `current` or `PWG-3`.
    Sym(String),
    /// A field name, such as `:state`, without the colon.
    Kw(String),
    /// `#t` or `#f`.
    Bool(bool),
    /// A tagged literal such as `#u8"chora:…"`: the tag and the string.
    Lit { tag: String, body: String },
}

/// Parse exactly one top-level s-expression; trailing content is an error.
pub fn parse(src: &str) -> Result<Sx> {
    let mut p = Parser {
        b: src.as_bytes(),
        src,
        i: 0,
        line: 1,
    };
    p.skip_ws();
    let sx = p.expr()?;
    p.skip_ws();
    if p.i != p.b.len() {
        bail!("line {}: content after the top-level form", p.line);
    }
    Ok(sx)
}

struct Parser<'a> {
    b: &'a [u8],
    src: &'a str,
    i: usize,
    line: usize,
}

fn is_delim(c: u8) -> bool {
    c.is_ascii_whitespace() || matches!(c, b'(' | b')' | b'"' | b';')
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'\n' => {
                    self.line += 1;
                    self.i += 1;
                }
                c if c.is_ascii_whitespace() => self.i += 1,
                b';' => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn expr(&mut self) -> Result<Sx> {
        let Some(&c) = self.b.get(self.i) else {
            bail!("line {}: unexpected end of input", self.line);
        };
        match c {
            b'(' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.b.get(self.i) {
                        None => bail!("line {}: unclosed `(`", self.line),
                        Some(b')') => {
                            self.i += 1;
                            return Ok(Sx::List(items));
                        }
                        Some(_) => items.push(self.expr()?),
                    }
                }
            }
            b')' => bail!("line {}: unexpected `)`", self.line),
            b'"' => Ok(Sx::Str(self.string()?)),
            b'#' => {
                let start = self.i + 1;
                let mut j = start;
                while j < self.b.len() && !is_delim(self.b[j]) {
                    j += 1;
                }
                let tag = &self.src[start..j];
                if j < self.b.len() && self.b[j] == b'"' && !tag.is_empty() {
                    self.i = j;
                    let body = self.string()?;
                    return Ok(Sx::Lit {
                        tag: tag.to_string(),
                        body,
                    });
                }
                self.i = j;
                match tag {
                    "t" => Ok(Sx::Bool(true)),
                    "f" => Ok(Sx::Bool(false)),
                    _ => bail!("line {}: unknown `#{tag}`", self.line),
                }
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len() && !is_delim(self.b[self.i]) {
                    self.i += 1;
                }
                let tok = &self.src[start..self.i];
                if let Some(k) = tok.strip_prefix(':') {
                    if k.is_empty() {
                        bail!("line {}: empty field name", self.line);
                    }
                    Ok(Sx::Kw(k.to_string()))
                } else {
                    Ok(Sx::Sym(tok.to_string()))
                }
            }
        }
    }

    /// A string starting at the `"` under the cursor.
    fn string(&mut self) -> Result<String> {
        let open = self.line;
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.b.get(self.i) else {
                bail!("line {open}: unclosed string");
            };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => match self.b.get(self.i + 1) {
                    Some(b'"') => {
                        out.push('"');
                        self.i += 2;
                    }
                    Some(b'\\') => {
                        out.push('\\');
                        self.i += 2;
                    }
                    _ => bail!("line {}: unsupported escape in a string", self.line),
                },
                _ => {
                    let ch = self.src[self.i..].chars().next().expect("in bounds");
                    if ch == '\n' {
                        self.line += 1;
                    }
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }
}

/// A form split into its head symbol, its `:field value` pairs (in order)
/// and its sub-clauses (in order). Fails on a field without a value, a value
/// without a field, a repeated field, or a form that is not `(head …)`.
#[derive(Debug)]
pub struct Form<'a> {
    pub head: &'a str,
    pub fields: Vec<(&'a str, &'a Sx)>,
    pub clauses: Vec<&'a Sx>,
}

impl<'a> Form<'a> {
    pub fn of(sx: &'a Sx) -> Result<Self> {
        let Sx::List(items) = sx else {
            bail!("expected a `(form …)`");
        };
        let Some(Sx::Sym(head)) = items.first() else {
            bail!("a form must start with its name");
        };
        let mut fields: Vec<(&str, &Sx)> = Vec::new();
        let mut clauses = Vec::new();
        let mut it = items[1..].iter();
        while let Some(x) = it.next() {
            match x {
                Sx::Kw(k) => {
                    let Some(v) = it.next() else {
                        bail!("({head} …): :{k} has no value");
                    };
                    if matches!(v, Sx::Kw(_) | Sx::List(_)) {
                        bail!("({head} …): :{k} has no value");
                    }
                    if fields.iter().any(|(f, _)| f == k) {
                        bail!("({head} …): :{k} appears twice");
                    }
                    fields.push((k, v));
                }
                Sx::List(_) => clauses.push(x),
                _ => bail!("({head} …): a value without a field name"),
            }
        }
        Ok(Self {
            head,
            fields,
            clauses,
        })
    }

    pub fn field(&self, name: &str) -> Option<&'a Sx> {
        self.fields
            .iter()
            .find(|(f, _)| *f == name)
            .map(|(_, v)| *v)
    }

    pub fn str(&self, name: &str) -> Result<Option<&'a str>> {
        match self.field(name) {
            None => Ok(None),
            Some(Sx::Str(s)) => Ok(Some(s)),
            Some(_) => bail!("({} …): :{name} must be a string", self.head),
        }
    }

    pub fn sym(&self, name: &str) -> Result<Option<&'a str>> {
        match self.field(name) {
            None => Ok(None),
            Some(Sx::Sym(s)) => Ok(Some(s)),
            Some(_) => bail!("({} …): :{name} must be a symbol", self.head),
        }
    }

    /// Sub-clauses whose head is `name`.
    pub fn clauses_named(&self, name: &str) -> Vec<&'a Sx> {
        self.clauses
            .iter()
            .copied()
            .filter(
                |c| matches!(c, Sx::List(v) if matches!(v.first(), Some(Sx::Sym(h)) if h == name)),
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_deed_atoms() {
        let sx =
            parse("; comment\n(d :a \"x \\\"q\\\" λ\" :b sym :c #t :d #u8\"chora:o\" (e :f #f))")
                .unwrap();
        let f = Form::of(&sx).unwrap();
        assert_eq!(f.head, "d");
        assert_eq!(f.str("a").unwrap(), Some("x \"q\" λ"));
        assert_eq!(f.sym("b").unwrap(), Some("sym"));
        assert_eq!(f.field("c"), Some(&Sx::Bool(true)));
        assert_eq!(
            f.field("d"),
            Some(&Sx::Lit {
                tag: "u8".into(),
                body: "chora:o".into()
            })
        );
        assert_eq!(f.clauses_named("e").len(), 1);
    }

    #[test]
    fn refuses_malformed_input() {
        for bad in [
            "(a",
            "(a))",
            "(a \"x)",
            "(a :b)",
            "(a :b :c 1)",
            "(a :b 1 :b 2)",
            "(a stray)",
            "(a #z)",
            "(a \"\\n\")",
            "(a) (b)",
        ] {
            let r = parse(bad).and_then(|sx| Form::of(&sx).map(|_| ()));
            assert!(r.is_err(), "accepted {bad:?}");
        }
    }
}
