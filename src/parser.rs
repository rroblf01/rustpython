use crate::ast::*;
use crate::token::{Lexer, Token};

pub struct Parser {
    lexer: Lexer,
    current: Token,
    peeked: Option<Token>,
    /// Start span of the current token (recorded at scan time, in
    /// lockstep with `current`). Unlike the lexer's `tok_start`,
    /// this never goes stale from lookahead scans, so wrapper starts
    /// are always exact.
    current_start: (usize, usize),
    /// Start span of the peeked lookahead token, if any.
    peeked_start: Option<(usize, usize)>,
    /// End span of the current token (recorded at scan time, in lockstep
    /// with `current`/`peeked`).
    current_end: (usize, usize),
    /// End span of the peeked lookahead token, if any.
    peeked_end: Option<(usize, usize)>,
    /// End span of the most recently consumed token — expression node
    /// ends (see `Expr::Located`). Updated on every `next()` from the
    /// outgoing current token's end.
    prev_end: (usize, usize),
    /// Start positions of consumed f/t-string openers, as a stack:
    /// nested strings parse (and pop) before outer ones resume, so
    /// LIFO order always matches. Read by `parse_fstring` /
    /// `parse_tstring` for the JoinedStr/TemplateStr start (the
    /// opener's own token position, which plain `tok_start` has
    /// already moved past by then).
    string_starts: Vec<(usize, usize)>,
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
    // Interpolated-string nesting depth (mirrors CPython's
    // MAXFSTRINGLEVEL=150): `f"{"f"{"..."}"}"` 160 deep is a
    // SyntaxError, while sequential f-strings must not accumulate
    // (decremented on success, like suite_depth's single-level use).
    fstring_depth: usize,
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
    ($self:expr, $expected:expr) => {{
        // A pending lexer error surfaces with its own message (and any
        // `L<line>:<col>:` position it carries) rather than wrapped as
        // "Expected ..., got LexerError(...)" — mirrors CPython, where
        // tokenizer errors propagate verbatim.
        if let Token::LexerError(msg) = &$self.current {
            Err(msg.clone())
        } else {
            Err(format!(
                "L{}:{}: Expected {}, got {}",
                $self.lexer.get_line_col().0,
                $self.lexer.get_line_col().1,
                $expected,
                $self.current
            ))
        }
    }};
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
    pub fn new(source: &str, eval_mode: bool) -> Self {
        let mut lexer = Lexer::new(source, eval_mode);
        let first = lexer.next_token();
        let first_start = lexer.tok_start();
        let first_end = lexer.tok_end();
        Parser {
            lexer,
            current: first,
            peeked: None,
            current_start: first_start,
            peeked_start: None,
            current_end: first_end,
            peeked_end: None,
            // Deliberately NOT `first_end`: the no-consumption
            // passthrough in expression wrappers compares against
            // this, and a single-token node (e.g. bare `a`) must
            // count as consuming.
            prev_end: (1, 1),
            string_starts: Vec::new(),
            unary_depth: 0,
            suite_depth: 0,
            fstring_depth: 0,
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
        let (tok, start, end) = match self.peeked.take() {
            Some(t) => (
                t,
                self.peeked_start.take().unwrap_or((1, 1)),
                self.peeked_end.take().unwrap_or((1, 1)),
            ),
            None => {
                let t = self.lexer.next_token();
                let s = self.lexer.tok_start();
                let e = self.lexer.tok_end();
                (t, s, e)
            }
        };
        // Record f/t-string openers for their wrappers (start travels
        // with the token here, immune to later lookahead scans).
        if matches!(tok, Token::FStringStart | Token::TStringStart) {
            self.string_starts.push(start);
        }
        self.prev_end = self.current_end;
        self.current_end = end;
        self.current_start = start;
        std::mem::replace(&mut self.current, tok)
    }

    fn peek(&mut self) -> &Token {
        if self.peeked.is_none() {
            self.peeked = Some(self.lexer.next_token());
            self.peeked_start = Some(self.lexer.tok_start());
            self.peeked_end = Some(self.lexer.tok_end());
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
            // Statement start line = current token start
            // (live lexer position already ran ahead over
            // eagerly scanned multiline strings).
            let line = self.current_start.0;
            let stmt = self.parse_stmt()?;
            self.push_tracked_stmt(&mut stmts, line, stmt);
            // Handle semicolon-separated statements on the same line: `while i: i = 0; continue`
            while self.eat(&Token::Semicolon) {
                if self.at(&Token::Dedent) || self.at(&Token::EndOfFile) {
                    break;
                }
                // Statement start line = current token start
                // (live lexer position already ran ahead over
                // eagerly scanned multiline strings).
                let line = self.current_start.0;
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
    let mut parser = Parser::new(source, true);
    // CPython's eval mode accepts a comma-separated test list (a tuple
    // display with optional trailing comma). Trailing tokens are plain
    // "invalid syntax" (mirrors CPython; `f'{0}' = x` must match that).
    // (No starred elements: real CPython raises SyntaxError for those.)
    // A statement-only keyword in first position (`del`, `return`,
    // `import`, ...) is also plain "invalid syntax", not a precise
    // expression error (mirrors CPython's `eval("del x")`).
    if matches!(
        &parser.current,
        Token::Del
            | Token::Return
            | Token::Raise
            | Token::Import
            | Token::From
            | Token::Class
            | Token::Def
            | Token::Async
            | Token::Global
            | Token::Nonlocal
            | Token::Pass
            | Token::Break
            | Token::Continue
            | Token::Assert
            | Token::If
            | Token::For
            | Token::While
            | Token::With
            | Token::Try
    ) {
        return Err("invalid syntax".to_string());
    }
    // A leading lexer error surfaces raw (mirrors CPython: `eval("'")`
    // reports "unterminated string literal ...", not wrapped as
    // "Expected expression, got LexerError(...)").
    if let Token::LexerError(msg) = &parser.current {
        return Err(msg.clone());
    }
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
        // A tokenizer error downstream (e.g. an unterminated string past
        // the first `=`) preempts the trailing-tokens error (mirrors
        // CPython, whose tokenizer runs ahead: `eval("ä = '''x")`
        // reports the unterminated string, not "invalid syntax").
        // (Includes one already pending as the current token.)
        if let Token::LexerError(msg) = &parser.current {
            return Err(msg.clone());
        }
        loop {
            parser.next();
            if let Token::LexerError(msg) = &parser.current {
                return Err(msg.clone());
            }
            if parser.at(&Token::EndOfFile) {
                break;
            }
        }
        return Err("invalid syntax".to_string());
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
                let error = Parser::new(&source, false).parse_program().unwrap_err();
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
            let error = Parser::new(source, false).parse_program().unwrap_err();
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
            Parser::new(source, false).parse_program().unwrap();
        }
    }

    #[test]
    fn parenthesized_patterns_are_grouping_not_sequences() {
        use super::{MatchCase, Pattern, Program, Stmt};
        fn first_pattern(source: &str) -> Pattern {
            let Program::Module(stmts) = Parser::new(source, false).parse_program().unwrap() else {
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
