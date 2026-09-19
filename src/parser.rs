use crate::ast::*;
use crate::token::{Lexer, Token};

pub struct Parser {
    lexer: Lexer,
    current: Token,
    peeked: Option<Token>,
    // >0 while parsing the body of an `async def` — gates `async for`/
    // `async with`/`await` (SyntaxError when used at top level, matching
    // CPython; see the emit sites in parse_stmt/parse_unary).
    async_depth: usize,
    // >0 while parsing a generator-expression body (`(x for x in ...)`).
    // `await` inside a genexpr is legal even in a sync function (it becomes
    // an async generator) — test_asyncgen's `make_arange` relies on it.
    genexpr_depth: usize,
    // `compile(..., flags=ast.PyCF_ALLOW_TOP_LEVEL_AWAIT)` relaxes the
    // above — top-level `await`/`async for`/`async with` become legal.
    pub(crate) allow_top_level_await: bool,
    // `compile(..., flags=__future__.CO_FUTURE_BARRY_AS_BDFL)` switches
    // `<>`/`!=` handling (barry_as_FLUFL; test_flufl).
    pub(crate) barry_as_bdfl: bool,
    // Guards `parse_unary`'s self-recursion (`-`/`+`/`~` chains) against a
    // native stack overflow on pathological input — real CPython's own PEG
    // parser has an equivalent C-stack-depth counter and raises `MemoryError:
    // too complex` past it, rather than crashing (its own `test_syntax.py`
    // explicitly tests for exactly this: `compile("-" * 100000 + "4", ...)`
    // must raise `MemoryError`, not abort the process). Without this guard,
    // this interpreter's own recursive-descent `parse_unary` (which calls
    // itself directly, once per leading `-`/`+`/`~`) genuinely overflowed
    // the native stack on that same input — confirmed via the exact repro,
    // crashing with a real OS-level "stack overflow, aborting" abort, not a
    // catchable Rust panic or Python exception.
    unary_depth: usize,
    // Nesting depth of "single-line compound-statement suite" parsing (the
    // `if x: a; b; c` / `def f(): a; b; c` form) — see `parse_block`'s own
    // doc comment and `expect_newline_or_eof`'s use of this field for the
    // full story. A counter (not a bool) in case a single-line suite's own
    // body itself starts another single-line suite (`if True: if False: a
    // = 1`), though that's a rare style choice, not the common trigger.
    suite_depth: usize,
    // Stack of the enclosing function's parameter names, used to reject
    // `global`/`nonlocal` applied to a parameter (test_syntax's
    // test_global_param_err_first / test_nonlocal_param_err_first).
    fn_params_stack: Vec<std::collections::HashSet<String>>,
    // Per-function names bound/used/annotated so far, to reject
    // `global`/`nonlocal` after prior use in the same scope
    // (CPython symtable's Global_kind checks). Pushed/popped in
    // lockstep with `fn_params_stack` in `parse_function_def`.
    pub(crate) fn_scope_stack: Vec<FnScopeTracker>,
    // Depth of nested class definitions — `nonlocal`/`global` inside a
    // class body refers to the enclosing *function* scope, not the class
    // itself (class scopes are not function scopes). Without this, the
    // param-vs-nonlocal check in `parse_nonlocal` incorrectly rejects
    // `def f(x): class c: nonlocal x` which CPython allows (test_scope
    // testNonLocalClass). Incremented in parse_class while parsing the
    // class body.
    class_depth: usize,
    // True when the most recently parsed atom was a parenthesized group.
    // `and`/`or` chains flatten into one BoolOp (CPython AST shape) but
    // must NOT merge across parens: `(x and y) and z` stays nested while
    // `x and y and z` is flat. The chain loops reset this before each
    // operand and read it after; the paren-group arm in parse_atom sets
    // it (after its inner parse, so inner chains still flatten).
    group_paren: bool,
}

const MAX_UNARY_DEPTH: usize = 2000;

macro_rules! unexpected_token {
    ($self:expr, $expected:expr) => {
        Err(format!(
            "L{}:{}: Expected {}, got {}",
            $self.lexer.get_line_col().0,
            $self.lexer.get_line_col().1,
            $expected,
            $self.current
        ))
    };
}

// --- Submodules (proof-of-concept split) ---
// Proposed full split (each <1k lines):
//   parser.rs        ~265  — Parser struct, helpers, parse_program, try_parse
//   parser/stmt.rs   ~843  — statement dispatch, simple stmts, args, block
//   parser/compound.rs ~599 — def/class/if/while/for/with/try/match/type_alias
//   parser/pattern.rs ~289 — match-case pattern parsing
//   parser/expr.rs    ~355 — expression precedence
//   parser/import.rs  ~117 — import handling
//   parser/primary.rs ~427 — primary/slice parsing
//   parser/atom.rs   ~931 — atom/fstring/lambda/yield
mod expr;
pub use expr::*;
mod import;
pub use import::*;
mod pattern;
pub use pattern::*;
mod scope;
pub use scope::*;
mod stmt;
pub use stmt::*;
mod compound;
pub use compound::*;
mod primary;
pub use primary::*;
mod atom;
pub use atom::*;

impl Parser {
    pub fn new(source: &str) -> Self {
        let mut lexer = Lexer::new(source);
        let first = lexer.next_token();
        Parser {
            lexer,
            current: first,
            peeked: None,
            unary_depth: 0,
            suite_depth: 0,
            async_depth: 0,
            genexpr_depth: 0,
            allow_top_level_await: false,
            barry_as_bdfl: false,
            fn_params_stack: Vec::new(),
            fn_scope_stack: Vec::new(),
            class_depth: 0,
            group_paren: false,
        }
    }

    fn next(&mut self) -> Token {
        let tok = self
            .peeked
            .take()
            .unwrap_or_else(|| self.lexer.next_token());
        std::mem::replace(&mut self.current, tok)
    }

    fn peek(&mut self) -> &Token {
        if self.peeked.is_none() {
            self.peeked = Some(self.lexer.next_token());
        }
        self.peeked.as_ref().unwrap()
    }

    /// Disambiguates the `match` soft keyword from `match` used as a plain
    /// identifier (e.g. `match, rest = x.split()` or `match = re.match(...)`).
    /// A real match-statement's header always ends with a bare `:` immediately
    /// before the line's NEWLINE, at bracket depth 0 — no other statement
    /// starting with a NAME token has that shape (annotated assignment's `:`
    /// appears right after the target, not at the end). We scan ahead with a
    /// cloned lexer so this is non-destructive to actual parse position.
    fn looks_like_match_stmt(&mut self) -> bool {
        let mut tok = self.peek().clone();
        let mut lexer = self.lexer.clone();
        let mut depth: i32 = 0;
        loop {
            match tok {
                Token::LeftParen | Token::LeftBracket | Token::LeftBrace => depth += 1,
                Token::RightParen | Token::RightBracket | Token::RightBrace => depth -= 1,
                Token::Newline | Token::EndOfFile => return false,
                Token::Colon if depth == 0 => {
                    // Must be followed directly by Newline (top-level match header)
                    let next = lexer.next_token();
                    return matches!(next, Token::Newline);
                }
                Token::Equal if depth == 0 => return false,
                _ => {}
            }
            tok = lexer.next_token();
        }
    }

    /// Disambiguates `with (` starting a PEP 617 parenthesized multi-item
    /// group (`with (cm1, cm2 as x):`, letting the items span multiple
    /// lines with trailing commas — real code, e.g. CPython's own test
    /// suite: `with (support.foo(), support.Bar() as sw):`) from `(` simply
    /// being the start of a single context-expression that happens to be
    /// parenthesized (`with (a + b).open():`, `with (some_cm):`). Called
    /// with the current token already `(`: scans ahead (non-destructively,
    /// cloned lexer — same technique as `looks_like_match_stmt`) for a
    /// top-level `,` or `as` before the matching `)`. Either one only makes
    /// sense inside a with-items group (a bare tuple/grouped-expr can't
    /// contain `as`, and `with (a, b):`'s comma is overwhelmingly intended
    /// as two context managers under the 3.10+ syntax, not the legacy
    /// tuple-as-one-CM reading) — so finding either is decisive. Finding
    /// neither means the parens belong to the expression itself, and must
    /// be left for `parse_expr` to consume normally.
    fn looks_like_parenthesized_with_items(&mut self) -> bool {
        let mut lexer = self.lexer.clone();
        let mut depth: i32 = 0;
        loop {
            let tok = lexer.next_token();
            match tok {
                Token::LeftParen | Token::LeftBracket | Token::LeftBrace => depth += 1,
                Token::RightParen if depth == 0 => return false,
                Token::RightParen | Token::RightBracket | Token::RightBrace => depth -= 1,
                Token::Newline | Token::EndOfFile => return false,
                Token::Comma | Token::As if depth == 0 => return true,
                _ => {}
            }
        }
    }

    fn at(&self, tok: &Token) -> bool {
        self.current == *tok
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.at(tok) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: &Token) -> Result<(), String> {
        if self.eat(tok) {
            Ok(())
        } else {
            unexpected_token!(self, tok)
        }
    }

    fn expect_name(&mut self) -> Result<String, String> {
        match &self.current {
            Token::Name(s) => {
                let s = s.clone();
                self.next();
                Ok(s)
            }
            // `_` is a regular identifier everywhere except match-statement
            // wildcard patterns (which are parsed separately) — e.g. the
            // common `from x import gettext_lazy as _` idiom.
            Token::Underscore => {
                self.next();
                Ok("_".to_string())
            }
            _ => unexpected_token!(self, "NAME"),
        }
    }

    // ---- Program ----

    /// Parse a single expression (for `ast.parse(..., mode="eval")`).
    /// Rejects trailing tokens after the expression.
    pub fn parse_single_expr(&mut self) -> Result<Expr, String> {
        while self.at(&Token::Newline) {
            self.next();
        }
        let e = self.parse_expr()?;
        while self.at(&Token::Newline) {
            self.next();
        }
        if !self.at(&Token::EndOfFile) {
            let (line, col) = self.lexer.get_line_col();
            return Err(format!("L{}:{}: invalid syntax", line, col));
        }
        Ok(e)
    }

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut stmts = Vec::new();
        while !self.at(&Token::EndOfFile) {
            while self.at(&Token::Newline) {
                self.next();
            }
            if self.at(&Token::EndOfFile) {
                break;
            }
            // A stray Indent/Dedent at module level is an indentation error.
            if self.at(&Token::Indent) || self.at(&Token::Dedent) {
                let (line, col) = self.lexer.get_line_col();
                return Err(format!("L{}:{}: unexpected indent", line, col));
            }
            let line = self.lexer.get_line_col().0;
            let stmt = self.parse_stmt()?;
            self.push_tracked_stmt(&mut stmts, line, stmt);
            // Handle semicolon-separated statements on the same line: `while i: i = 0; continue`
            while self.eat(&Token::Semicolon) {
                if self.at(&Token::Dedent) || self.at(&Token::EndOfFile) {
                    break;
                }
                let line = self.lexer.get_line_col().0;
                let stmt = self.parse_stmt()?;
                self.push_tracked_stmt(&mut stmts, line, stmt);
            }
        }
        Ok(Program::Module(stmts))
    }

    // ---- Statements ----
}

/// Try to parse a source string as a single expression.
/// Used by the REPL to detect expression statements whose value should
/// be displayed via sys.displayhook instead of being discarded by POP_TOP.
pub fn try_parse_as_expression(source: &str) -> Result<Program, String> {
    let mut parser = Parser::new(source);
    // CPython's eval mode accepts a comma-separated test list (a tuple
    // display with optional trailing comma) — plain `parse_expr` alone
    // rejects `eval("1, 2")` with "extra tokens after expression". (No
    // starred elements: real CPython raises SyntaxError for those.)
    let first = parser.parse_expr()?;
    let mut exprs = vec![first];
    let mut saw_comma = false;
    while parser.eat(&Token::Comma) {
        saw_comma = true;
        if parser.at(&Token::EndOfFile)
            || parser.at(&Token::Newline)
            || parser.at(&Token::Semicolon)
        {
            break;
        }
        exprs.push(parser.parse_expr()?);
    }
    // Consume trailing newlines
    while parser.eat(&Token::Newline) || parser.eat(&Token::Semicolon) {}
    if !parser.at(&Token::EndOfFile) {
        return Err("extra tokens after expression".to_string());
    }
    let expr = if exprs.len() == 1 && !saw_comma {
        exprs.pop().unwrap()
    } else {
        crate::ast::Expr::Tuple(exprs)
    };
    Ok(Program::Expression(Box::new(expr)))
}

#[cfg(test)]
mod tests {
    use super::Parser;

    #[test]
    fn parameter_declaration_error_locations() {
        for keyword in ["global", "nonlocal"] {
            for (source, line, column) in [
                (format!("def f(x):\n    {keyword} x\n"), 2, 5),
                (format!("def f(x): {keyword} x\n"), 1, 11),
                (format!("def f(x):\n  {keyword} x\n"), 2, 3),
                (format!("def f(x):\n    {keyword} y, x\n"), 2, 5),
            ] {
                let error = Parser::new(&source).parse_program().unwrap_err();
                assert_eq!(
                    error,
                    format!("L{line}:{column}: name 'x' is parameter and {keyword}"),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn global_after_prior_binding_errors() {
        for (source, message) in [
            (
                "def f():\n    x = 1\n    global x\n",
                "L3:5: name 'x' is assigned to before global declaration",
            ),
            (
                "def f():\n    print(x)\n    global x\n",
                "L3:5: name 'x' is used prior to global declaration",
            ),
            (
                "def f():\n    x: int\n    global x\n",
                "L3:5: annotated name 'x' can't be global",
            ),
            (
                "def f():\n    x = 1\n    nonlocal x\n",
                "L3:5: name 'x' is assigned to before nonlocal declaration",
            ),
            (
                "def f():\n    print(x)\n    nonlocal x\n",
                "L3:5: name 'x' is used prior to nonlocal declaration",
            ),
            (
                "def f():\n    x: int\n    nonlocal x\n",
                "L3:5: annotated name 'x' can't be nonlocal",
            ),
        ] {
            let error = Parser::new(source).parse_program().unwrap_err();
            assert_eq!(error, message, "{source}");
        }
    }

    #[test]
    fn global_before_binding_is_legal() {
        for source in [
            "def f():\n    global x\n    x = 1\n",
            "def f():\n    global x\n    print(x)\n",
            "def f():\n    global x\n    global x\n",
            "x = 1\nglobal x\n",
        ] {
            Parser::new(source).parse_program().unwrap();
        }
    }

    #[test]
    fn parenthesized_patterns_are_grouping_not_sequences() {
        use super::{MatchCase, Pattern, Program, Stmt};
        fn first_pattern(source: &str) -> Pattern {
            let Program::Module(stmts) = Parser::new(source).parse_program().unwrap() else {
                panic!("not a module: {source}");
            };
            let [Stmt::Located(_, body)] = stmts.as_slice() else {
                panic!("one stmt: {source}");
            };
            let Stmt::Match { cases, .. } = body.as_ref() else {
                panic!("match stmt: {source}");
            };
            let [MatchCase { pattern, .. }] = cases.as_slice() else {
                panic!("one case: {source}");
            };
            pattern.clone()
        }
        // `(p)` groups: as-capture and or-alternatives pass through.
        assert!(matches!(
            first_pattern("match x:\n    case (0 as z):\n        pass\n"),
            Pattern::MatchAs { .. }
        ));
        assert!(matches!(
            first_pattern("match x:\n    case (1 | 2):\n        pass\n"),
            Pattern::MatchOr(_)
        ));
        // Comma makes it a sequence; brackets always do.
        assert!(matches!(
            first_pattern("match x:\n    case (0,):\n        pass\n"),
            Pattern::MatchSequence(_)
        ));
        assert!(matches!(
            first_pattern("match x:\n    case [0]:\n        pass\n"),
            Pattern::MatchSequence(_)
        ));
        assert!(matches!(
            first_pattern("match x:\n    case ():\n        pass\n"),
            Pattern::MatchSequence(_)
        ));
    }
}
