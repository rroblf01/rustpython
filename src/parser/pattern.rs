use std::collections::HashSet;

use crate::ast::*;
use crate::token::Token;

use super::Parser;

impl Parser {
    pub(crate) fn parse_match_cases(&mut self) -> Result<Vec<MatchCase>, String> {
        // Grammar: 'match' subject ':' NEWLINE INDENT case_block+ DEDENT —
        // there are TWO nested indentation levels here: this outer one
        // (wrapping the whole set of `case` clauses) and, separately, each
        // case's own body (opened/closed by parse_block() below, which
        // already consumes exactly its own one closing Dedent). Previously
        // this loop ate Indent/Dedent tokens unconditionally while looking
        // for the next `case`, which — once the last case's body ended —
        // also swallowed the Dedent(s) belonging to whatever *encloses* the
        // match statement (the function/module body), silently truncating
        // everything after it from the parse tree.
        let mut cases = Vec::new();
        self.eat(&Token::Newline);
        let had_indent = self.eat(&Token::Indent);
        loop {
            while self.eat(&Token::Newline) {}
            if !matches!(&self.current, Token::Name(n) if n == "case") {
                break;
            }
            self.next(); // consume "case" keyword
            let mut pattern = self.parse_pattern()?;
            // Open sequence pattern: `case 0, *x:`  (comma-separated without parens)
            if self.eat(&Token::Comma) {
                let mut patterns = vec![pattern];
                loop {
                    while self.at(&Token::Newline) {
                        self.next();
                    }
                    if self.at(&Token::Colon) || self.at(&Token::If) {
                        break;
                    }
                    if self.eat(&Token::Star) {
                        let name = if matches!(&self.current, Token::Name(_)) {
                            Some(self.expect_name()?)
                        } else if self.eat(&Token::Underscore) {
                            Some("_".to_string())
                        } else {
                            None
                        };
                        patterns.push(Pattern::MatchStar { name });
                    } else {
                        patterns.push(self.parse_pattern()?);
                    }
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
                pattern = Pattern::MatchSequence(patterns);
            }
            let guard = if self.eat(&Token::If) {
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            self.expect(&Token::Colon)?;
            let body = self.parse_block()?;
            validate_pattern(&pattern)?;
            cases.push(MatchCase {
                pattern,
                guard,
                body,
            });
        }
        if had_indent {
            // Exactly one Dedent, matching the Indent consumed above — any
            // further dedents belong to the enclosing scope.
            self.eat(&Token::Dedent);
        }
        validate_match_cases_reachable(&cases)?;
        Ok(cases)
    }

    pub(crate) fn parse_pattern(&mut self) -> Result<Pattern, String> {
        self.parse_or_pattern()
    }

    pub(crate) fn parse_or_pattern(&mut self) -> Result<Pattern, String> {
        // `as` binds looser than `|`: `0 | 1 | 2 as z` is `(0 | 1 | 2) as z`
        // (CPython AST: As(Or, z)), while `(0 as z) | 1` keeps the inner as.
        // So alternatives are closed patterns (no trailing `as`); one
        // optional `as NAME` wraps the whole or afterwards.
        let mut patterns = vec![self.parse_closed_pattern()?];
        while self.eat(&Token::Pipe) {
            patterns.push(self.parse_closed_pattern()?);
        }
        let mut pattern = if patterns.len() == 1 {
            patterns.into_iter().next().unwrap()
        } else {
            Pattern::MatchOr(patterns)
        };
        if self.eat(&Token::As) {
            let name = Some(self.expect_name()?);
            pattern = Pattern::MatchAs {
                pattern: Some(Box::new(pattern)),
                name,
            };
        }
        Ok(pattern)
    }

    /// A single or-alternative: everything `parse_as_pattern` accepts
    /// except a trailing `as NAME` (handled one level up, looser than `|`).
    pub(crate) fn parse_closed_pattern(&mut self) -> Result<Pattern, String> {
        self.parse_literal_pattern()
    }

    pub(crate) fn parse_as_pattern(&mut self) -> Result<Pattern, String> {
        // Kept for compatibility; full patterns go through parse_or_pattern
        // (which handles the looser trailing `as`). Direct callers get the
        // closed form.
        self.parse_closed_pattern()
    }

    pub(crate) fn parse_literal_pattern(&mut self) -> Result<Pattern, String> {
        if self.at(&Token::Underscore) {
            self.next();
            return Ok(Pattern::MatchAs {
                pattern: None,
                name: None,
            });
        }
        if self.eat(&Token::Star) {
            let name = if matches!(&self.current, Token::Name(_)) {
                Some(self.expect_name()?)
            } else if self.eat(&Token::Underscore) {
                Some("_".to_string())
            } else {
                None
            };
            return Ok(Pattern::MatchStar { name });
        }
        if matches!(&self.current, Token::Name(_)) {
            let name = self.expect_name()?;
            if self.at(&Token::LeftParen) {
                return self.parse_class_pattern(Expr::Name(name));
            }
            // A dotted name (`case Format.STRING:`, `case some.deep.CONST:`)
            // is a "value pattern" — matched by equality against the
            // referenced value — NOT a capture pattern, per real Python's
            // match-statement grammar (only a bare, undotted name captures).
            // Without this check every qualified-constant case clause
            // (extremely common — enum members, module-level constants)
            // was silently treated as an always-matching capture binding
            // instead of an equality check. Real code: CPython 3.14's own
            // `annotationlib.py`, `case Format.STRING:` inside a real
            // stdlib function.
            if self.at(&Token::Dot) {
                let mut expr = Expr::Name(name);
                while self.eat(&Token::Dot) {
                    let attr = self.expect_name()?;
                    expr = Expr::Attribute {
                        value: Box::new(expr),
                        attr,
                    };
                }
                // A dotted CLASS pattern (`case unittest.TestResult():`,
                // `case pkg.mod.Foo(x, y=1):`) — the dotted chain names a
                // class, not a value, when immediately followed by `(`.
                // Missing this check dropped straight into the value-
                // pattern return below, which left the `(` unconsumed and
                // the following `:` unreachable, breaking every dotted-
                // class match arm (confirmed real: `Lib/test/test_regrtest
                // ...`'s `case unittest.TestResult():`).
                if self.at(&Token::LeftParen) {
                    return self.parse_class_pattern(expr);
                }
                return Ok(Pattern::MatchValue(Box::new(expr)));
            }
            if name == "_" {
                return Ok(Pattern::MatchAs {
                    pattern: None,
                    name: None,
                });
            }
            return Ok(Pattern::MatchAs {
                pattern: None,
                name: Some(name),
            });
        }
        if self.at(&Token::LeftParen) || self.at(&Token::LeftBracket) {
            return self.parse_sequence_pattern();
        }
        if self.at(&Token::LeftBrace) {
            return self.parse_mapping_pattern();
        }
        // `None`/`True`/`False` are singletons: CPython matches them by
        // IDENTITY (`is`), not `==` — `match 0: case False:` must MISS
        // (`0 == False` is True but `0 is False` is not), likewise
        // `match 1: case True:`. The compiler's MatchSingleton arm emits
        // COMPARE_OP IS for exactly this.
        if self.at(&Token::None) {
            self.next();
            return Ok(Pattern::MatchSingleton("None".to_string()));
        }
        if self.at(&Token::True) {
            self.next();
            return Ok(Pattern::MatchSingleton("True".to_string()));
        }
        if self.at(&Token::False) {
            self.next();
            return Ok(Pattern::MatchSingleton("False".to_string()));
        }
        // A literal/value pattern (`case 1:`, `case -1:`, `case 1+2j:`,
        // `case "x":`) must NOT consume `|` here — `|` inside a `case`
        // clause is ALWAYS the or-pattern separator (`parse_or_pattern`'s
        // own job, one level up), never a bitwise-or expression. This used
        // to call `parse_or_expr` (the full expression grammar, all the way
        // up through logical `or`/`and`/comparison/bitwise-or), which
        // greedily swallowed the `|` as `Expr::BinOp` before
        // `parse_or_pattern`'s `while self.eat(&Token::Pipe)` loop ever saw
        // it — so `case 'Y' | 'y' | 'G':` compiled as ONE `MatchValue`
        // wrapping the literal expression `'Y' | 'y' | 'G'`, evaluated at
        // MATCH TIME as a real binary `|` between string values (raising
        // `TypeError: unsupported operand type(s) for |: 'str' and 'str'`
        // the instant any such case ran) instead of three separate
        // alternatives. Confirmed general, not `_strptime.py`-specific: any
        // `case A | B:` where `A`/`B` don't happen to support `|` (almost
        // everything except numbers) was completely broken. Stopping at
        // `parse_bitwise_xor` (one precedence level tighter, skipping
        // `parse_bitwise_or`, logical `or`/`and`/`not`, and comparisons —
        // none of which real Python's own literal-pattern grammar permits
        // anyway) still correctly parses negative numbers and complex
        // literals (`-5`, `1+2j`), just without swallowing a trailing `|`.
        let expr = self.parse_bitwise_xor()?;
        Ok(Pattern::MatchValue(Box::new(expr)))
    }

    pub(crate) fn parse_class_pattern(&mut self, cls: Expr) -> Result<Pattern, String> {
        self.expect(&Token::LeftParen)?;
        let mut patterns = Vec::new();
        let mut kwd_attrs = Vec::new();
        let mut kwd_patterns = Vec::new();
        if !self.at(&Token::RightParen) {
            loop {
                if matches!(&self.current, Token::Name(_)) && self.peek() == &Token::Equal {
                    kwd_attrs.push(self.expect_name()?);
                    self.expect(&Token::Equal)?;
                    kwd_patterns.push(self.parse_pattern()?);
                } else {
                    patterns.push(self.parse_pattern()?);
                }
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
        }
        self.expect(&Token::RightParen)?;
        Ok(Pattern::MatchClass {
            cls: Box::new(cls),
            patterns,
            kwd_attrs,
            kwd_patterns,
        })
    }

    pub(crate) fn parse_sequence_pattern(&mut self) -> Result<Pattern, String> {
        let open = if self.eat(&Token::LeftBracket) {
            "["
        } else {
            self.expect(&Token::LeftParen)?;
            "("
        };
        let mut patterns = Vec::new();
        if open == "(" && self.at(&Token::RightParen) {
            self.next();
            return Ok(Pattern::MatchSequence(patterns));
        }
        // `(p)` is just grouping around `p` (e.g. `(0 as z)`, `(1 | 2)`),
        // NOT a one-element sequence — only `(p,)`/`(p, q)` (comma present)
        // is a sequence pattern. Without this, every parenthesized
        // or/capture pattern compiled as a sequence gate and cleanly
        // failed everything (PATMA-034/081 class).
        let mut saw_comma = false;
        loop {
            if open == "[" && self.at(&Token::RightBracket) {
                break;
            }
            if open == "(" && self.at(&Token::RightParen) {
                break;
            }
            if self.eat(&Token::Star) {
                let name = if matches!(&self.current, Token::Name(_)) {
                    Some(self.expect_name()?)
                } else if self.eat(&Token::Underscore) {
                    Some("_".to_string())
                } else {
                    None
                };
                patterns.push(Pattern::MatchStar { name });
            } else {
                patterns.push(self.parse_pattern()?);
            }
            if !self.eat(&Token::Comma) {
                break;
            }
            saw_comma = true;
        }
        let close = if open == "[" {
            Token::RightBracket
        } else {
            Token::RightParen
        };
        self.expect(&close)?;
        if open == "(" && !saw_comma && patterns.len() == 1 {
            let only = patterns.into_iter().next().unwrap();
            if matches!(only, Pattern::MatchStar { .. }) {
                // `case (*x):` needs the comma (`case (*x,):`) — real
                // CPython rejects it as invalid syntax.
                return Err("invalid syntax".to_string());
            }
            return Ok(only);
        }
        Ok(Pattern::MatchSequence(patterns))
    }

    pub(crate) fn parse_mapping_pattern(&mut self) -> Result<Pattern, String> {
        self.expect(&Token::LeftBrace)?;
        let mut keys = Vec::new();
        let mut rest = None;
        if !self.at(&Token::RightBrace) {
            loop {
                if self.eat(&Token::DoubleStar) {
                    rest = Some(self.expect_name()?);
                    // `**rest` must be last: at most one trailing comma
                    // may follow (`{**rest,}` is legal; `{**a, **b}` and
                    // `{**rest, "k": v}` are not).
                    if self.eat(&Token::Comma) && !self.at(&Token::RightBrace) {
                        return Err("invalid syntax".to_string());
                    }
                    break;
                }
                keys.push(self.parse_literal_pattern()?);
                self.expect(&Token::Colon)?;
                keys.push(self.parse_pattern()?);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
        }
        self.expect(&Token::RightBrace)?;
        Ok(Pattern::MatchMapping { keys, rest })
    }
}

// ---- match-pattern static validation (CPython symtable/compile checks) ----
// `TestSyntaxErrors` (44 tests) asserts these are all SyntaxErrors. The
// checks only need to RAISE (assert_syntax_error doesn't match messages),
// but messages mirror CPython's where cheap.

/// Cross-case check: an unguarded irrefutable case makes later cases dead.
pub(crate) fn validate_match_cases_reachable(cases: &[MatchCase]) -> Result<(), String> {
    for (i, case) in cases.iter().enumerate() {
        if case.guard.is_none() && i + 1 < cases.len() {
            if let Some(msg) = irrefutable_message(&case.pattern) {
                return Err(msg);
            }
        }
    }
    Ok(())
}

/// If the pattern always matches, why (for the unreachable message).
fn irrefutable_message(pat: &Pattern) -> Option<String> {
    match pat {
        Pattern::MatchAs {
            pattern: None,
            name: None,
        } => Some("wildcard makes remaining patterns unreachable".to_string()),
        Pattern::MatchAs {
            pattern: None,
            name: Some(n),
        } if n != "_" => Some(format!(
            "name capture '{n}' makes remaining patterns unreachable"
        )),
        Pattern::MatchAs {
            pattern: Some(inner),
            ..
        } => irrefutable_message(inner),
        Pattern::MatchOr(alts) => alts.iter().find_map(irrefutable_message),
        _ => None,
    }
}

/// Per-pattern checks + recursion. Duplicate/alternative binding rules are
/// enforced by `bound_names` (called first); the rest is structural.
pub(crate) fn validate_pattern(pat: &Pattern) -> Result<(), String> {
    bound_names(pat)?;
    match pat {
        Pattern::MatchOr(alts) => {
            for alt in alts {
                validate_pattern(alt)?;
            }
            for alt in &alts[..alts.len().saturating_sub(1)] {
                if let Some(msg) = irrefutable_message(alt) {
                    return Err(msg);
                }
            }
            Ok(())
        }
        Pattern::MatchSequence(elts) => {
            if elts
                .iter()
                .filter(|p| matches!(p, Pattern::MatchStar { .. }))
                .count()
                > 1
            {
                return Err("multiple starred names in sequence pattern".to_string());
            }
            for e in elts {
                validate_pattern(e)?;
            }
            Ok(())
        }
        Pattern::MatchMapping { keys, rest } => {
            if rest.as_deref() == Some("_") {
                return Err("invalid syntax".to_string());
            }
            let mut seen: Vec<PatKey> = Vec::new();
            for chunk in keys.chunks(2) {
                let [key_pat, val_pat] = chunk else {
                    return Err("invalid syntax".to_string());
                };
                let key_expr = match key_pat {
                    Pattern::MatchValue(expr) => expr,
                    _ => {
                        return Err(
                            "mapping pattern keys may only match literals and attribute lookups"
                                .to_string(),
                        );
                    }
                };
                validate_value_expr(key_expr)?;
                if let Some(k) = pat_key_of(key_expr) {
                    if let Some(dup) = seen.iter().find(|p| patkey_eq(p, &k)) {
                        return Err(format!(
                            "mapping pattern checks duplicate key ({})",
                            patkey_repr(dup)
                        ));
                    }
                    seen.push(k);
                }
                validate_pattern(val_pat)?;
            }
            Ok(())
        }
        Pattern::MatchClass {
            patterns,
            kwd_attrs,
            kwd_patterns,
            ..
        } => {
            let mut seen_attrs = HashSet::new();
            for a in kwd_attrs {
                if !seen_attrs.insert(a.clone()) {
                    return Err(format!("attribute name repeated in class pattern: {a}"));
                }
            }
            for p in patterns.iter().chain(kwd_patterns.iter()) {
                validate_pattern(p)?;
            }
            Ok(())
        }
        Pattern::MatchAs { pattern, name } => {
            if pattern.is_some() && name.as_deref() == Some("_") {
                return Err("cannot use '_' as a target".to_string());
            }
            if let Some(inner) = pattern {
                validate_pattern(inner)?;
            }
            Ok(())
        }
        Pattern::MatchStar { .. } => Ok(()),
        Pattern::MatchValue(expr) => validate_value_expr(expr),
        Pattern::MatchSingleton(_) => Ok(()),
    }
}

/// All names a pattern binds (source order, `_` excluded), enforcing
/// single-assignment within a pattern and same-bindings across `|`
/// alternatives. Errors use CPython's texts.
fn bound_names(pat: &Pattern) -> Result<Vec<String>, String> {
    fn insert(out: &mut Vec<String>, seen: &mut HashSet<String>, n: &str) -> Result<(), String> {
        if n == "_" {
            return Ok(());
        }
        if !seen.insert(n.to_string()) {
            return Err(format!("multiple assignments to name '{n}' in pattern"));
        }
        out.push(n.to_string());
        Ok(())
    }
    fn go(pat: &Pattern, out: &mut Vec<String>, seen: &mut HashSet<String>) -> Result<(), String> {
        match pat {
            Pattern::MatchAs { pattern, name } => {
                if let Some(inner) = pattern {
                    go(inner, out, seen)?;
                }
                if let Some(n) = name {
                    insert(out, seen, n)?;
                }
                Ok(())
            }
            Pattern::MatchStar { name } => {
                if let Some(n) = name {
                    insert(out, seen, n)?;
                }
                Ok(())
            }
            Pattern::MatchSequence(elts) => {
                for e in elts {
                    go(e, out, seen)?;
                }
                Ok(())
            }
            Pattern::MatchMapping { keys, rest } => {
                for chunk in keys.chunks(2) {
                    if let [_, val] = chunk {
                        go(val, out, seen)?;
                    }
                }
                if let Some(n) = rest {
                    insert(out, seen, n)?;
                }
                Ok(())
            }
            Pattern::MatchClass {
                patterns,
                kwd_patterns,
                ..
            } => {
                for p in patterns.iter().chain(kwd_patterns.iter()) {
                    go(p, out, seen)?;
                }
                Ok(())
            }
            Pattern::MatchOr(alts) => {
                let mut first: Option<HashSet<String>> = None;
                for alt in alts {
                    let mut a_out = Vec::new();
                    let mut a_seen = HashSet::new();
                    go(alt, &mut a_out, &mut a_seen)?;
                    match &first {
                        None => first = Some(a_seen),
                        Some(prev) if *prev != a_seen => {
                            return Err("alternative patterns bind different names".to_string());
                        }
                        _ => {}
                    }
                }
                // Merge the common set for outer dup tracking (`([x]|[x], x)`
                // still double-binds x). Sorted for determinism.
                if let Some(set) = first {
                    let mut names: Vec<String> = set.into_iter().collect();
                    names.sort();
                    for n in names {
                        insert(out, seen, &n)?;
                    }
                }
                Ok(())
            }
            Pattern::MatchValue(_) | Pattern::MatchSingleton(_) => Ok(()),
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    go(pat, &mut out, &mut seen)?;
    Ok(out)
}

/// Value patterns must be literals or dotted attribute lookups (complex
/// literals included, with real/imaginary side rules).
fn validate_value_expr(expr: &Expr) -> Result<(), String> {
    match expr {
        Expr::Constant(
            Constant::None
            | Constant::Bool(_)
            | Constant::Int(_)
            | Constant::Float(_)
            | Constant::String(_)
            | Constant::Bytes(_)
            | Constant::Ellipsis,
        ) => Ok(()),
        Expr::Constant(Constant::Complex { .. }) => Ok(()),
        Expr::Attribute { .. } => {
            let mut e = expr;
            loop {
                match e {
                    Expr::Attribute { value, .. } => e = value,
                    Expr::Name(_) => return Ok(()),
                    _ => {
                        return Err(
                            "patterns may only match literals and attribute lookups".to_string()
                        );
                    }
                }
            }
        }
        Expr::UnaryOp {
            op: UnaryOp::USub | UnaryOp::UAdd,
            operand,
        } => validate_signed_number(operand),
        Expr::BinOp {
            left,
            op: Operator::Add | Operator::Sub,
            right,
        } => validate_complex_parts(left, right),
        _ => Err("patterns may only match literals and attribute lookups".to_string()),
    }
}

/// `-5`/`+5` (and signed imaginary): operand must be a plain number.
fn validate_signed_number(expr: &Expr) -> Result<(), String> {
    match expr {
        Expr::Constant(Constant::Int(_) | Constant::Float(_) | Constant::Complex { .. }) => Ok(()),
        _ => Err("patterns may only match literals and attribute lookups".to_string()),
    }
}

/// `real +/- imag`: real must be real-numbered, imag imaginary.
fn validate_complex_parts(real: &Expr, imag: &Expr) -> Result<(), String> {
    if !is_real_number(real) {
        return Err("real number required in complex literal".to_string());
    }
    if !is_imaginary_number(imag) {
        return Err("imaginary number required in complex literal".to_string());
    }
    Ok(())
}

fn is_real_number(expr: &Expr) -> bool {
    match expr {
        Expr::Constant(Constant::Int(_) | Constant::Float(_)) => true,
        Expr::UnaryOp {
            op: UnaryOp::USub | UnaryOp::UAdd,
            operand,
        } => is_real_number(operand),
        _ => false,
    }
}

fn is_imaginary_number(expr: &Expr) -> bool {
    match expr {
        Expr::Constant(Constant::Complex { .. }) => true,
        Expr::UnaryOp {
            op: UnaryOp::USub | UnaryOp::UAdd,
            operand,
        } => is_imaginary_number(operand),
        _ => false,
    }
}

/// Canonical mapping-key literal for duplicate detection across the
/// numeric tower (`0 == False == 0.0 == 0j`) and equal strings.
#[derive(Debug, Clone)]
enum PatKey {
    None,
    Bool(bool),
    Int(i128),
    Float(f64),
    Complex(f64, f64),
    Str(String),
    Bytes(Vec<u8>),
}

fn parse_py_int(s: &str) -> Option<i128> {
    let t: String = s.chars().filter(|c| *c != '_').collect();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i128::from_str_radix(h, 16).ok()
    } else if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        i128::from_str_radix(b, 2).ok()
    } else if let Some(o) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        i128::from_str_radix(o, 8).ok()
    } else {
        t.parse::<i128>().ok()
    }
}

fn pat_key_of(expr: &Expr) -> Option<PatKey> {
    match expr {
        Expr::Constant(Constant::None) => Some(PatKey::None),
        Expr::Constant(Constant::Bool(b)) => Some(PatKey::Bool(*b)),
        Expr::Constant(Constant::Int(s)) => parse_py_int(s).map(PatKey::Int),
        Expr::Constant(Constant::Float(s)) => s
            .chars()
            .filter(|c| *c != '_')
            .collect::<String>()
            .parse::<f64>()
            .ok()
            .map(PatKey::Float),
        Expr::Constant(Constant::Complex { real, imag }) => {
            let r = real.replace('_', "").parse::<f64>().ok()?;
            let i = imag.replace('_', "").parse::<f64>().ok()?;
            Some(PatKey::Complex(r, i))
        }
        Expr::Constant(Constant::String(s)) => Some(PatKey::Str(s.to_string())),
        Expr::Constant(Constant::Bytes(b)) => Some(PatKey::Bytes(b.clone())),
        Expr::UnaryOp {
            op: UnaryOp::USub,
            operand,
        } => match pat_key_of(operand)? {
            PatKey::Int(i) => Some(PatKey::Int(-i)),
            PatKey::Float(f) => Some(PatKey::Float(-f)),
            PatKey::Complex(r, i) => Some(PatKey::Complex(-r, -i)),
            _ => None,
        },
        Expr::UnaryOp {
            op: UnaryOp::UAdd,
            operand,
        } => pat_key_of(operand),
        Expr::BinOp {
            left,
            op: Operator::Add | Operator::Sub,
            right,
        } => {
            // Complex key literal (validated before this runs): real +/- imag*j.
            let rv = patkey_real_value(left)?;
            let iv = patkey_imag_value(right)?;
            let op_add = matches!(
                expr,
                Expr::BinOp {
                    op: Operator::Add,
                    ..
                }
            );
            Some(PatKey::Complex(rv, if op_add { iv } else { -iv }))
        }
        _ => None,
    }
}

fn patkey_real_value(expr: &Expr) -> Option<f64> {
    match pat_key_of(expr)? {
        PatKey::Int(i) => Some(i as f64),
        PatKey::Float(f) => Some(f),
        _ => None,
    }
}

fn patkey_imag_value(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Constant(Constant::Complex { real, imag }) => {
            let r: f64 = real.replace('_', "").parse().ok()?;
            let i: f64 = imag.replace('_', "").parse().ok()?;
            if r != 0.0 {
                return None;
            }
            Some(i)
        }
        Expr::UnaryOp {
            op: UnaryOp::USub,
            operand,
        } => patkey_imag_value(operand).map(|v| -v),
        Expr::UnaryOp {
            op: UnaryOp::UAdd,
            operand,
        } => patkey_imag_value(operand),
        _ => None,
    }
}

fn patkey_eq(a: &PatKey, b: &PatKey) -> bool {
    fn num(k: &PatKey) -> Option<(f64, f64)> {
        match k {
            PatKey::Int(i) => Some((*i as f64, 0.0)),
            PatKey::Float(f) => Some((*f, 0.0)),
            PatKey::Complex(r, i) => Some((*r, *i)),
            PatKey::Bool(true) => Some((1.0, 0.0)),
            PatKey::Bool(false) => Some((0.0, 0.0)),
            _ => None,
        }
    }
    match (num(a), num(b)) {
        (Some((ar, ai)), Some((br, bi))) => ar == br && ai == bi,
        _ => match (a, b) {
            (PatKey::None, PatKey::None) => true,
            (PatKey::Str(x), PatKey::Str(y)) => x == y,
            (PatKey::Bytes(x), PatKey::Bytes(y)) => x == y,
            _ => false,
        },
    }
}

fn patkey_repr(k: &PatKey) -> String {
    match k {
        PatKey::None => "None".to_string(),
        PatKey::Bool(true) => "True".to_string(),
        PatKey::Bool(false) => "False".to_string(),
        PatKey::Int(i) => i.to_string(),
        PatKey::Float(f) => {
            if f.fract() == 0.0 {
                format!("{f:.1}")
            } else {
                f.to_string()
            }
        }
        PatKey::Complex(r, i) => format!("{r}+{i}j"),
        PatKey::Str(s) => format!("'{s}'"),
        PatKey::Bytes(b) => format!("{b:?}"),
    }
}
