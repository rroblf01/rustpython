use std::fmt;

pub type Ident = String;

#[derive(Debug, Clone, PartialEq)]
pub enum Program {
    Module(Vec<Stmt>),
    Expression(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    FunctionDef {
        name: Ident,
        args: Vec<Arg>,
        body: Vec<Stmt>,
        decorator_list: Vec<Expr>,
        returns: Option<Box<Expr>>,
        is_async: bool,
    },
    ClassDef {
        name: Ident,
        bases: Vec<Expr>,
        keywords: Vec<Keyword>,
        body: Vec<Stmt>,
        decorator_list: Vec<Expr>,
    },
    Return(Option<Box<Expr>>),
    Delete(Vec<Expr>),
    Assign {
        targets: Vec<Expr>,
        value: Box<Expr>,
    },
    AugAssign {
        target: Box<Expr>,
        op: Operator,
        value: Box<Expr>,
    },
    AnnAssign {
        target: Box<Expr>,
        annotation: Box<Expr>,
        value: Option<Box<Expr>>,
    },
    For {
        target: Box<Expr>,
        iter: Box<Expr>,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
        is_async: bool,
    },
    While {
        test: Box<Expr>,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    If {
        test: Box<Expr>,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    With {
        items: Vec<WithItem>,
        body: Vec<Stmt>,
        is_async: bool,
    },
    Match {
        subject: Box<Expr>,
        cases: Vec<MatchCase>,
    },
    Raise {
        exc: Option<Box<Expr>>,
        cause: Option<Box<Expr>>,
    },
    Try {
        body: Vec<Stmt>,
        handlers: Vec<ExceptHandler>,
        handlers_star: Vec<ExceptStar>,
        orelse: Vec<Stmt>,
        finalbody: Vec<Stmt>,
    },
    Assert {
        test: Box<Expr>,
        msg: Option<Box<Expr>>,
    },
    Import(Vec<Alias>),
    ImportFrom {
        module: Option<Ident>,
        names: Vec<Alias>,
        level: Option<u32>,
    },
    Global(Vec<Ident>),
    Nonlocal(Vec<Ident>),
    Expr(Box<Expr>),
    Pass,
    Break,
    Continue,
    TypeAlias {
        name: Ident,
        type_params: Vec<Ident>,
        value: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub arg: Ident,
    pub annotation: Option<Box<Expr>>,
    pub is_vararg: bool,
    pub is_kwarg: bool,
    pub is_posonlyarg: bool,
    pub default: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyword {
    pub arg: Option<Ident>,
    pub value: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WithItem {
    pub context_expr: Box<Expr>,
    pub optional_vars: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub pattern: Pattern,
    pub guard: Option<Box<Expr>>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExceptHandler {
    pub typ: Option<Box<Expr>>,
    pub name: Option<Ident>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExceptStar {
    pub typ: Option<Box<Expr>>,
    pub name: Option<Ident>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alias {
    pub name: Ident,
    pub asname: Option<Ident>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    MatchValue(Box<Expr>),
    // Compiler (compiler.rs) fully handles this, but the parser never emits
    // it yet — `case None`/`True`/`False` currently parse as MatchValue
    // (equality) instead of identity comparison. Keep for when that's wired up.
    #[allow(dead_code)]
    MatchSingleton(String),
    MatchSequence(Vec<Pattern>),
    MatchMapping {
        keys: Vec<Pattern>,
        rest: Option<Ident>,
    },
    MatchClass {
        cls: Box<Expr>,
        patterns: Vec<Pattern>,
        kwd_attrs: Vec<Ident>,
        kwd_patterns: Vec<Pattern>,
    },
    MatchStar {
        name: Option<Ident>,
    },
    MatchAs {
        pattern: Option<Box<Pattern>>,
        name: Option<Ident>,
    },
    MatchOr(Vec<Pattern>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    BoolOp {
        op: BoolOp,
        values: Vec<Expr>,
    },
    NamedExpr {
        target: Box<Expr>,
        value: Box<Expr>,
    },
    BinOp {
        left: Box<Expr>,
        op: Operator,
        right: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Lambda {
        args: Vec<Arg>,
        body: Box<Expr>,
    },
    IfExp {
        test: Box<Expr>,
        body: Box<Expr>,
        orelse: Box<Expr>,
    },
    Dict {
        keys: Vec<Option<Expr>>,
        values: Vec<Expr>,
    },
    Set(Vec<Expr>),
    ListComp {
        elt: Box<Expr>,
        generators: Vec<Comprehension>,
    },
    SetComp {
        elt: Box<Expr>,
        generators: Vec<Comprehension>,
    },
    DictComp {
        key: Box<Expr>,
        value: Box<Expr>,
        generators: Vec<Comprehension>,
    },
    GeneratorExp {
        elt: Box<Expr>,
        generators: Vec<Comprehension>,
    },
    Await(Box<Expr>),
    Yield(Option<Box<Expr>>),
    YieldFrom(Box<Expr>),
    Compare {
        left: Box<Expr>,
        ops: Vec<CmpOp>,
        comparators: Vec<Expr>,
    },
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
        keywords: Vec<Keyword>,
    },
    FString(Vec<FStringPart>),
    // Superseded by FString/FStringPart; the compiler still has a handler
    // for it but the parser never constructs one. Kept for compatibility
    // with the existing compiler.rs arm rather than ripping both out.
    #[allow(dead_code)]
    JoinedStr(Vec<Expr>),
    Constant(Constant),
    Attribute {
        value: Box<Expr>,
        attr: Ident,
    },
    Subscript {
        value: Box<Expr>,
        slice: Box<Expr>,
    },
    Starred(Box<Expr>),
    Name(Ident),
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Slice {
        lower: Option<Box<Expr>>,
        upper: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    String(String),
    Expr {
        expr: Box<Expr>,
        conversion: u8,  // 0=none, 1=repr(!r), 2=str(!s), 3=ascii(!a)
        format_spec: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comprehension {
    pub target: Box<Expr>,
    pub iter: Box<Expr>,
    pub ifs: Vec<Expr>,
    pub is_async: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    None,
    Bool(bool),
    Int(String),
    Float(String),
    Complex { real: String, imag: String },
    String(String),
    Bytes(Vec<u8>),
    Ellipsis,
}

impl Constant {
    pub fn int_from_str(s: &str) -> Self {
        Constant::Int(s.to_string())
    }
    pub fn float_from_str(s: &str) -> Self {
        Constant::Float(s.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoolOp {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operator {
    Add,
    Sub,
    Mult,
    Div,
    FloorDiv,
    Mod,
    Pow,
    LShift,
    RShift,
    BitOr,
    BitXor,
    BitAnd,
    MatMult,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Invert,
    Not,
    UAdd,
    USub,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    Is,
    IsNot,
    In,
    NotIn,
}

impl fmt::Display for Operator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operator::Add => write!(f, "+"),
            Operator::Sub => write!(f, "-"),
            Operator::Mult => write!(f, "*"),
            Operator::Div => write!(f, "/"),
            Operator::FloorDiv => write!(f, "//"),
            Operator::Mod => write!(f, "%"),
            Operator::Pow => write!(f, "**"),
            Operator::LShift => write!(f, "<<"),
            Operator::RShift => write!(f, ">>"),
            Operator::BitOr => write!(f, "|"),
            Operator::BitXor => write!(f, "^"),
            Operator::BitAnd => write!(f, "&"),
            Operator::MatMult => write!(f, "@"),
        }
    }
}

impl fmt::Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CmpOp::Eq => write!(f, "=="),
            CmpOp::NotEq => write!(f, "!="),
            CmpOp::Lt => write!(f, "<"),
            CmpOp::LtE => write!(f, "<="),
            CmpOp::Gt => write!(f, ">"),
            CmpOp::GtE => write!(f, ">="),
            CmpOp::Is => write!(f, "is"),
            CmpOp::IsNot => write!(f, "is not"),
            CmpOp::In => write!(f, "in"),
            CmpOp::NotIn => write!(f, "not in"),
        }
    }
}
