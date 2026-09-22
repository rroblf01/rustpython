use crate::ast::*;
use crate::token::Token;

use super::{Parser, MAX_UNARY_DEPTH};

impl Parser {
    pub(crate) fn parse_expr(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_expr_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_expr_inner(&mut self) -> Result<Expr, String> {
        let expr = self.parse_conditional_expr()?;
        // Walrus operator (:=) — named expressions, allowed at top expr level
        // e.g. `if spec := getattr(...):`
        if self.eat(&Token::Walrus) {
            let value = self.parse_expr()?;
            Ok(Expr::NamedExpr {
                target: Box::new(expr),
                value: Box::new(value),
            })
        } else {
            Ok(expr)
        }
    }

    pub(crate) fn parse_conditional_expr(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_conditional_expr_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_conditional_expr_inner(&mut self) -> Result<Expr, String> {
        let expr = self.parse_or_expr()?;
        if self.eat(&Token::If) {
            let test = self.parse_conditional_expr()?;
            self.expect(&Token::Else)?;
            let orelse = self.parse_conditional_expr()?;
            Ok(Expr::IfExp {
                test: Box::new(test),
                body: Box::new(expr),
                orelse: Box::new(orelse),
            })
        } else {
            Ok(expr)
        }
    }

    pub(crate) fn parse_or_expr(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_or_expr_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_or_expr_inner(&mut self) -> Result<Expr, String> {
        self.group_paren = false;
        let mut expr = self.parse_and_expr()?;
        let mut expr_paren = std::mem::replace(&mut self.group_paren, false);
        while self.eat(&Token::Or) {
            self.group_paren = false;
            let right = self.parse_and_expr()?;
            let _right_paren = std::mem::replace(&mut self.group_paren, false);
            // Flatten chained `or` into a single BoolOp (CPython AST
            // shape), but never across parens (`(x or y) or z` nests).
            // Peel positions: the accumulator may be `Located` once
            // expression functions stamp spans (rebuilt below).
            while let Expr::Located { node, .. } = expr {
                expr = *node;
            }
            match expr {
                Expr::BoolOp {
                    op: BoolOp::Or,
                    mut values,
                } if !expr_paren => {
                    values.push(right);
                    expr = Expr::BoolOp {
                        op: BoolOp::Or,
                        values,
                    };
                }
                _ => {
                    expr = Expr::BoolOp {
                        op: BoolOp::Or,
                        values: vec![expr, right],
                    };
                }
            }
            expr_paren = false;
        }
        // A lone parenthesized operand stays marked for the outer level
        // (`(x or y) or z` must not flatten); a built chain never is.
        self.group_paren = expr_paren;
        Ok(expr)
    }

    pub(crate) fn parse_and_expr(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_and_expr_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_and_expr_inner(&mut self) -> Result<Expr, String> {
        self.group_paren = false;
        let mut expr = self.parse_not_expr()?;
        let mut expr_paren = std::mem::replace(&mut self.group_paren, false);
        while self.eat(&Token::And) {
            self.group_paren = false;
            let right = self.parse_not_expr()?;
            let _right_paren = std::mem::replace(&mut self.group_paren, false);
            // Flatten chained `and` (same rule as `or` above).
            // Peel positions (see `or` above).
            while let Expr::Located { node, .. } = expr {
                expr = *node;
            }
            match expr {
                Expr::BoolOp {
                    op: BoolOp::And,
                    mut values,
                } if !expr_paren => {
                    values.push(right);
                    expr = Expr::BoolOp {
                        op: BoolOp::And,
                        values,
                    };
                }
                _ => {
                    expr = Expr::BoolOp {
                        op: BoolOp::And,
                        values: vec![expr, right],
                    };
                }
            }
            expr_paren = false;
        }
        // A lone parenthesized operand stays marked for the outer level
        // (`(x and y) and z` must not flatten); a built chain never is.
        self.group_paren = expr_paren;
        Ok(expr)
    }

    pub(crate) fn parse_not_expr(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_not_expr_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_not_expr_inner(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Not) {
            let expr = self.parse_not_expr()?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(expr),
            })
        } else {
            self.parse_comparison()
        }
    }

    pub(crate) fn parse_comparison(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_comparison_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_comparison_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_bitwise_or()?;
        if self.at(&Token::Less)
            || self.at(&Token::Greater)
            || self.at(&Token::LessEqual)
            || self.at(&Token::GreaterEqual)
            || self.at(&Token::EqualEqual)
            || self.at(&Token::NotEqual)
            || self.at(&Token::LessGreater)
            || self.at(&Token::Is)
            || self.at(&Token::In)
            || self.at(&Token::Not)
        {
            let mut ops = Vec::new();
            let mut comparators = Vec::new();
            loop {
                let cmp_token = self.current.clone();
                let op = match cmp_token {
                    Token::Less => {
                        self.next();
                        CmpOp::Lt
                    }
                    Token::Greater => {
                        self.next();
                        CmpOp::Gt
                    }
                    Token::LessEqual => {
                        self.next();
                        CmpOp::LtE
                    }
                    Token::GreaterEqual => {
                        self.next();
                        CmpOp::GtE
                    }
                    Token::EqualEqual => {
                        self.next();
                        CmpOp::Eq
                    }
                    Token::NotEqual => {
                        let (line, col) = self.lexer.get_line_col();
                        self.next();
                        if self.barry_as_bdfl {
                            // With Barry as BDFL, `!=` is rejected (use
                            // `<>` instead).
                            return Err(format!(
                                "L{}:{}:with Barry as BDFL, use '<>' instead of '!='",
                                line,
                                col.saturating_sub(2)
                            ));
                        }
                        CmpOp::NotEq
                    }
                    Token::LessGreater => {
                        let (line, col) = self.lexer.get_line_col();
                        self.next();
                        if !self.barry_as_bdfl {
                            // `<>` is 2 chars; lexer col points just past it
                            // (test_flufl expects offset at the '<').
                            return Err(format!(
                                "L{}:{}:invalid syntax: '<>'",
                                line,
                                col.saturating_sub(2)
                            ));
                        }
                        CmpOp::NotEq
                    }
                    Token::Is => {
                        self.next();
                        if self.eat(&Token::Not) {
                            CmpOp::IsNot
                        } else {
                            CmpOp::Is
                        }
                    }
                    Token::In => {
                        self.next();
                        CmpOp::In
                    }
                    Token::Not => {
                        if self.peek() == &Token::In {
                            self.next();
                            self.next();
                            CmpOp::NotIn
                        } else {
                            break;
                        }
                    }
                    _ => break,
                };
                ops.push(op);
                comparators.push(self.parse_bitwise_or()?);
            }
            expr = Expr::Compare {
                left: Box::new(expr),
                ops,
                comparators,
            };
        }
        Ok(expr)
    }

    pub(crate) fn parse_bitwise_or(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_bitwise_or_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_bitwise_or_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_bitwise_xor()?;
        while self.eat(&Token::Pipe) {
            let right = self.parse_bitwise_xor()?;
            expr = Expr::BinOp {
                left: Box::new(expr),
                op: Operator::BitOr,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    pub(crate) fn parse_bitwise_xor(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_bitwise_xor_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_bitwise_xor_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_bitwise_and()?;
        while self.eat(&Token::Caret) {
            let right = self.parse_bitwise_and()?;
            expr = Expr::BinOp {
                left: Box::new(expr),
                op: Operator::BitXor,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    pub(crate) fn parse_bitwise_and(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_bitwise_and_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_bitwise_and_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_shift()?;
        while self.eat(&Token::Ampersand) {
            let right = self.parse_shift()?;
            expr = Expr::BinOp {
                left: Box::new(expr),
                op: Operator::BitAnd,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    pub(crate) fn parse_shift(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_shift_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_shift_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_term()?;
        loop {
            if self.eat(&Token::LeftShift) {
                let right = self.parse_term()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::LShift,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::RightShift) {
                let right = self.parse_term()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::RShift,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    pub(crate) fn parse_term(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_term_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_term_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_mul()?;
        loop {
            if self.eat(&Token::Plus) {
                let right = self.parse_mul()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Add,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Minus) {
                let right = self.parse_mul()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Sub,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    pub(crate) fn parse_mul(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_mul_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_mul_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_unary()?;
        loop {
            if self.eat(&Token::Star) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Mult,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Slash) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Div,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::DoubleSlash) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::FloorDiv,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Percent) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Mod,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::At) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::MatMult,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    pub(crate) fn parse_unary(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_unary_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_unary_inner(&mut self) -> Result<Expr, String> {
        let is_unary_prefix = matches!(self.current, Token::Plus | Token::Minus | Token::Tilde);
        if is_unary_prefix {
            self.unary_depth += 1;
            if self.unary_depth > MAX_UNARY_DEPTH {
                self.unary_depth -= 1;
                return Err("MemoryError: too complex".to_string());
            }
        }
        let result = if self.eat(&Token::Plus) {
            let expr = self.parse_unary()?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::UAdd,
                operand: Box::new(expr),
            })
        } else if self.eat(&Token::Minus) {
            let expr = self.parse_unary()?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::USub,
                operand: Box::new(expr),
            })
        } else if self.eat(&Token::Tilde) {
            let expr = self.parse_unary()?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::Invert,
                operand: Box::new(expr),
            })
        } else {
            self.parse_power()
        };
        if is_unary_prefix {
            self.unary_depth -= 1;
        }
        result
    }

    pub(crate) fn parse_power(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_power_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_power_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        if self.eat(&Token::DoubleStar) {
            let right = self.parse_unary()?;
            expr = Expr::BinOp {
                left: Box::new(expr),
                op: Operator::Pow,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }
}
