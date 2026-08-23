use std::cell::RefCell;
use std::fmt;

use crate::interner::StrId;
use crate::object::PyObjectRef;

// Some variants are reserved for planned work (register-based bytecode
// phase in ROADMAP-v2.md; match-statement opcodes with a VM handler
// but no compiler emission yet, per GAP_ANALYSIS.md) and aren't
// constructed anywhere yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types, dead_code)]
pub enum Opcode {
    // Actual opcodes matching CPython
    BINARY_OP = 42,
    BUILD_LIST = 44,
    BUILD_MAP = 45,
    BUILD_SET = 46,
    BUILD_SLICE = 47,
    BUILD_STRING = 48,
    BUILD_TUPLE = 49,
    CALL = 50,
    CALL_FUNCTION_EX = 4,
    CALL_KW = 53,
    COMPARE_OP = 54,
    CONTAINS_OP = 55,
    COPY = 57,
    COPY_FREE_VARS = 58,
    DELETE_FAST = 60,
    DELETE_NAME = 61,
    EXTENDED_ARG = 64,
    FOR_ITER = 65,
    GET_ITER = 67,
    IMPORT_FROM = 68,
    IMPORT_NAME = 69,
    IMPORT_STAR = 70,
    IS_OP = 71,
    JUMP_BACKWARD = 72,
    JUMP_FORWARD = 73,
    LIST_APPEND = 74,
    LIST_EXTEND = 75,
    LOAD_ATTR = 76,
    LOAD_CONST = 78,
    LOAD_DEREF = 79,
    LOAD_FAST = 80,
    LOAD_FROM_DICT_OR_GLOBALS = 87,
    LOAD_GLOBAL = 88,
    LOAD_NAME = 89,
    MAKE_CELL = 93,
    MAP_ADD = 94,
    POP_JUMP_IF_FALSE = 96,
    POP_JUMP_IF_NONE = 97,
    POP_JUMP_IF_NOT_NONE = 98,
    POP_JUMP_IF_TRUE = 99,
    RAISE_VARARGS = 100,
    RERAISE = 101,
    // Clears `active_exception` so a following RERAISE pops its operand from
    // the VALUE STACK instead (except* semantics: after the handlers, the
    // unmatched ExceptionGroup sits on the stack and must be re-raised, not
    // the original exception held in active_exception).
    CLEAR_EXCEPTION_INFO = 121,
    SEND = 102,
    SET_ADD = 103,
    SET_FUNCTION_ATTRIBUTE = 104,
    SET_UPDATE = 105,
    STORE_ATTR = 106,
    STORE_DEREF = 107,
    STORE_FAST = 108,
    DELETE_ATTR = 109,
    DELETE_SUBSCR = 110,
    STORE_GLOBAL = 111,
    STORE_NAME = 112,
    SWAP = 113,
    UNPACK_EX = 114,
    UNPACK_SEQUENCE = 115,
    YIELD_VALUE = 116,

    // These are in the special range but we use standard aliases
    RESUME = 128,
    JUMP = 257,
    POP_BLOCK = 262,
    SETUP_FINALLY = 264,
    SETUP_CLEANUP = 263,
    SETUP_WITH = 265,
    WITH_EXIT = 266,

    DUP_TOP = 26,
    POP_ITER = 28,
    STORE_SUBSCR = 36,
    LOAD_CLOSURE = 261,
    // Not in standard opcodes but we define:
    NOP = 25,
    POP_TOP = 29,
    PUSH_NULL = 31,
    RETURN_VALUE = 33,
    UNARY_NEGATIVE = 39,
    UNARY_NOT = 40,
    UNARY_INVERT = 38,
    UNARY_POSITIVE = 11,
    GET_LEN = 16,
    MATCH_MAPPING = 23,
    MATCH_SEQUENCE = 24,
    MATCH_KEYS = 22,
    CHECK_EXC_MATCH = 6,
    PUSH_EXC_INFO = 30,
    END_FOR = 9,
    GET_AITER = 14,
    GET_ANEXT = 15,
    GET_AWAITABLE = 66,
    CLEANUP_THROW = 7,
    END_SEND = 10,
    BEFORE_ASYNC_WITH = 0,
    CALL_INTRINSIC_1 = 51,
    CALL_INTRINSIC_2 = 52,
    FORMAT_SIMPLE = 12,
    FORMAT_WITH_SPEC = 13,
    CONVERT_VALUE = 56,
    LOAD_BUILD_CLASS = 19,
    LOAD_LOCALS = 20,
    MAKE_FUNCTION = 21,
    RETURN_GENERATOR = 32,
    SETUP_ANNOTATIONS = 34,
    POP_EXCEPT = 27,
    END_FINALLY = 117,
    ELSE = 118,
    POP_EXCEPT_AND_EXECUTE_FINALLY = 119,
    UNPACK_SEQUENCE_TWO_TUPLE = 218,
    // ── Register-based instructions (prefix 0xC0, non-standard) ──────
    // These encode virtual register operands instead of using a stack.
    // The upper 4 bits of `arg` encode dst register, lower encode src(s).
    REG_MOV = 0xC0,           // r[dst] = r[src]
    REG_LOAD_CONST = 0xC1,    // r[dst] = consts[arg2]
    REG_LOAD_FAST = 0xC2,     // r[dst] = fast_locals[arg2]
    REG_STORE_FAST = 0xC3,    // fast_locals[arg2] = r[src]
    REG_BINARY_OP = 0xC4,     // r[dst] = r[a] OP r[b]
    REG_LOAD_GLOBAL = 0xC5,   // r[dst] = globals/builtins[name_idx]
    REG_CALL = 0xC6,          // r[dst] = call(r[func], r[args...])
    REG_RETURN = 0xC7,        // return r[src]
    REG_JUMP_IF_FALSE = 0xC8, // if !r[src]: pc += offset
    REG_BUILD_LIST = 0xC9,    // r[dst] = [r[arg0], r[arg1], ...]

    // Custom opcodes for dict operations
    DICT_MERGE = 202, // Pop TOS (source dict) and merge into dict at TOS1

    // Pop TOS (a list) and push a tuple with the same elements — used to
    // build a tuple literal containing starred unpacking, e.g. (*a, *b),
    // which is built incrementally as a list (LIST_APPEND/LIST_EXTEND)
    // and then converted, matching CPython's own LIST_TO_TUPLE.
    LIST_TO_TUPLE = 203,

    // ExceptionGroup splitting for except* (PEP 654)
    // Like CHECK_EXC_MATCH but for except*: pops type + exc_dup + exc_orig,
    // splits ExceptionGroup into matched/unmatched subgroups, pushes
    // [unmatched_eg, matched_eg, True] on match or [exc_orig, False] on no match.
    CHECK_EXC_MATCH_STAR = 120,

    // ── Superinstructions (peephole-fused sequences) ─────────────────
    // Emitted by `superinstr::apply` after compilation. Each fuses a short
    // hot sequence into ONE dispatch; the handler advances ip past the
    // (NOP-filled) fused slots, so jump targets stay valid without any
    // remapping. Fused slots are never jump targets themselves.
    /// LOAD_FAST a, LOAD_FAST b, BINARY_OP op(non-inplace), STORE_FAST z
    /// arg = a | b<<8 | op<<16 | z<<24  (all < 256, else not fused)
    SUPER_FAST2_BIN = 124,
    /// LOAD_FAST a, LOAD_CONST c, BINARY_OP op(non-inplace), STORE_FAST z
    /// arg = a | c<<8 | op<<24  (a,z < 256; c < 65536, else not fused)
    SUPER_FASTC_BIN = 125,
    /// LOAD_FAST a, STORE_FAST z   arg = a | z<<16
    SUPER_FAST_MOV = 126,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum OpGroup {
    Load,
    Store,
    Delete,
    Arith,
    Call,
    Control,
    Build,
    Stack,
    Other,
}

impl Opcode {
    /// Categorize opcode into a dispatch group for faster matching.
    /// Each group is small enough (5-15 members) that LLVM generates
    /// a dense jump table instead of a binary search tree.
    pub fn group(self) -> OpGroup {
        use Opcode::*;
        match self {
            LOAD_FAST | LOAD_CONST | LOAD_GLOBAL | LOAD_NAME | LOAD_DEREF | LOAD_ATTR => {
                OpGroup::Load
            }
            STORE_FAST | STORE_NAME | STORE_GLOBAL | STORE_DEREF | STORE_ATTR | STORE_SUBSCR => {
                OpGroup::Store
            }
            DELETE_FAST | DELETE_NAME | DELETE_ATTR | DELETE_SUBSCR => OpGroup::Delete,
            BINARY_OP | COMPARE_OP | CONTAINS_OP | IS_OP | UNARY_NEGATIVE | UNARY_NOT
            | UNARY_INVERT | UNARY_POSITIVE => OpGroup::Arith,
            CALL | CALL_FUNCTION_EX | CALL_KW | PUSH_NULL => OpGroup::Call,
            JUMP_FORWARD | JUMP_BACKWARD | JUMP | POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE
            | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE | FOR_ITER => OpGroup::Control,
            BUILD_LIST | BUILD_TUPLE | BUILD_MAP | BUILD_SET | BUILD_SLICE | BUILD_STRING
            | LIST_APPEND | LIST_EXTEND | MAP_ADD | SET_ADD | SET_UPDATE | UNPACK_SEQUENCE
            | UNPACK_EX => OpGroup::Build,
            DUP_TOP | POP_TOP | COPY | SWAP | GET_ITER => OpGroup::Stack,
            RETURN_VALUE | YIELD_VALUE | RAISE_VARARGS | RERAISE | SEND => OpGroup::Control,
            _ => OpGroup::Other,
        }
    }

    pub fn from_u16(n: u16) -> Option<Opcode> {
        use Opcode::*;
        Some(match n {
            4 => CALL_FUNCTION_EX,
            42 => BINARY_OP,
            44 => BUILD_LIST,
            45 => BUILD_MAP,
            46 => BUILD_SET,
            47 => BUILD_SLICE,
            48 => BUILD_STRING,
            49 => BUILD_TUPLE,
            50 => CALL,
            51 => CALL_INTRINSIC_1,
            52 => CALL_INTRINSIC_2,
            53 => CALL_KW,
            54 => COMPARE_OP,
            55 => CONTAINS_OP,
            64 => EXTENDED_ARG,
            65 => FOR_ITER,
            67 => GET_ITER,
            68 => IMPORT_FROM,
            69 => IMPORT_NAME,
            70 => IMPORT_STAR,
            71 => IS_OP,
            72 => JUMP_BACKWARD,
            73 => JUMP_FORWARD,
            74 => LIST_APPEND,
            75 => LIST_EXTEND,
            76 => LOAD_ATTR,
            78 => LOAD_CONST,
            79 => LOAD_DEREF,
            80 => LOAD_FAST,
            87 => LOAD_FROM_DICT_OR_GLOBALS,
            88 => LOAD_GLOBAL,
            89 => LOAD_NAME,
            93 => MAKE_CELL,
            96 => POP_JUMP_IF_FALSE,
            97 => POP_JUMP_IF_NONE,
            98 => POP_JUMP_IF_NOT_NONE,
            99 => POP_JUMP_IF_TRUE,
            100 => RAISE_VARARGS,
            102 => SEND,
            103 => SET_ADD,
            104 => SET_FUNCTION_ATTRIBUTE,
            105 => SET_UPDATE,
            106 => STORE_ATTR,
            107 => STORE_DEREF,
            108 => STORE_FAST,
            109 => DELETE_ATTR,
            110 => DELETE_SUBSCR,
            111 => STORE_GLOBAL,
            112 => STORE_NAME,
            113 => SWAP,
            115 => UNPACK_SEQUENCE,
            114 => UNPACK_EX,
            116 => YIELD_VALUE,
            128 => RESUME,
            25 => NOP,
            29 => POP_TOP,
            31 => PUSH_NULL,
            33 => RETURN_VALUE,
            38 => UNARY_INVERT,
            39 => UNARY_NEGATIVE,
            40 => UNARY_NOT,
            11 => UNARY_POSITIVE,
            28 => DUP_TOP,
            0 => STORE_SUBSCR,
            261 => LOAD_CLOSURE,
            202 => DICT_MERGE,
            218 => UNPACK_SEQUENCE_TWO_TUPLE,
            120 => CHECK_EXC_MATCH_STAR,
            124 => SUPER_FAST2_BIN,
            125 => SUPER_FASTC_BIN,
            126 => SUPER_FAST_MOV,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instr {
    pub op: Opcode,
    pub arg: u32,
}

impl Instr {
    pub fn with_arg(op: Opcode, arg: u32) -> Self {
        Instr { op, arg }
    }
}

impl fmt::Display for Instr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.op)?;
        if self.arg != 0 || needs_arg(self.op) {
            write!(f, " {}", self.arg)?;
        }
        Ok(())
    }
}

pub(crate) fn needs_arg(op: Opcode) -> bool {
    use Opcode::*;
    matches!(
        op,
        BINARY_OP
            | BUILD_LIST
            | BUILD_MAP
            | BUILD_SET
            | BUILD_SLICE
            | BUILD_STRING
            | BUILD_TUPLE
            | CALL
            | CALL_FUNCTION_EX
            | CALL_KW
            | COMPARE_OP
            | CONTAINS_OP
            | COPY
            | COPY_FREE_VARS
            | DELETE_FAST
            | DELETE_NAME
            | EXTENDED_ARG
            | FOR_ITER
            | IMPORT_FROM
            | IMPORT_NAME
            | IMPORT_STAR
            | IS_OP
            | JUMP_BACKWARD
            | JUMP_FORWARD
            | LIST_APPEND
            | LIST_EXTEND
            | LOAD_ATTR
            | LOAD_CONST
            | LOAD_DEREF
            | LOAD_FAST
            | LOAD_FROM_DICT_OR_GLOBALS
            | LOAD_GLOBAL
            | LOAD_NAME
            | MAKE_CELL
            | MAP_ADD
            | POP_JUMP_IF_FALSE
            | POP_JUMP_IF_NONE
            | POP_JUMP_IF_NOT_NONE
            | POP_JUMP_IF_TRUE
            | RAISE_VARARGS
            | RERAISE
            | SEND
            | SET_ADD
            | SET_FUNCTION_ATTRIBUTE
            | SET_UPDATE
            | STORE_ATTR
            | STORE_DEREF
            | STORE_FAST
            | DELETE_ATTR
            | DELETE_SUBSCR
            | STORE_GLOBAL
            | STORE_NAME
            | SWAP
            | UNPACK_EX
            | UNPACK_SEQUENCE
            | YIELD_VALUE
            | RESUME
            | JUMP
            | POP_BLOCK
            | SETUP_FINALLY
            | SETUP_CLEANUP
            | SETUP_WITH
            | MAKE_FUNCTION
            | LOAD_BUILD_CLASS
            | PUSH_NULL
            | RETURN_VALUE
            | UNARY_NEGATIVE
            | UNARY_NOT
            | UNARY_INVERT
            | UNARY_POSITIVE
            | GET_LEN
            | MATCH_MAPPING
            | MATCH_SEQUENCE
            | MATCH_KEYS
            | CHECK_EXC_MATCH
            | PUSH_EXC_INFO
            | END_FOR
            | GET_AITER
            | GET_ANEXT
            | GET_AWAITABLE
            | CLEANUP_THROW
            | END_SEND
            | FORMAT_SIMPLE
            | FORMAT_WITH_SPEC
            | CONVERT_VALUE
            | LOAD_LOCALS
            | RETURN_GENERATOR
            | SETUP_ANNOTATIONS
            | POP_EXCEPT
            | UNPACK_SEQUENCE_TWO_TUPLE
            | DUP_TOP
            | STORE_SUBSCR
            | LOAD_CLOSURE
            | POP_ITER
            | DICT_MERGE
    )
}

#[derive(Debug, Clone)]
pub struct CodeObject {
    pub name: StrId,
    pub arg_count: usize,
    pub posonlyarg_count: usize,
    pub kwonlyarg_count: usize,
    pub nlocals: usize,
    pub instructions: Vec<Instr>,
    pub consts: Vec<ConstValue>,
    pub names: Vec<crate::interner::StrId>,
    pub varnames: Vec<crate::interner::StrId>,
    pub freevars: Box<Vec<String>>,
    pub cellvars: Box<Vec<String>>,
    pub filename: StrId,
    pub first_lineno: usize,
    pub flags: u16,
    pub vararg_name: Option<Box<String>>,
    pub kwarg_name: Option<Box<String>>,
    pub num_defaults: usize,
    /// Per keyword-only parameter (in order, length == kwonlyarg_count),
    /// whether it has a default value. Keyword-only defaults can't use the
    /// "trailing N have defaults" trick positional defaults use (kwonly
    /// params may have defaults in any order, e.g. `def f(*, a=1, b, c=2)`),
    /// so each slot is tracked individually. Values live in
    /// PyObject::Function.defaults right after the `num_defaults` positional
    /// ones, in the same left-to-right order as the `true` entries here.
    pub kwonly_defaults_mask: Box<Vec<bool>>,
    /// Lazily-populated cache of each `LOAD_CONST`'s already-parsed result,
    /// indexed by `consts` position. Without this, `LOAD_CONST` re-derives
    /// the actual `PyObjectRef` from scratch on EVERY execution of that
    /// instruction — for `ConstValue::Int(String)`/`Float(String)`, that
    /// means re-parsing the same string (underscore-cleaning, `0x`/`0o`/
    /// `0b` prefix detection, radix parsing) every single time, even though
    /// the same bytecode offset always parses the same source text to the
    /// same value; for `ConstValue::Code(Box<CodeObject>)` (a nested
    /// function/lambda's own compiled body), it means a full recursive
    /// clone of that nested `CodeObject` — including ITS OWN `consts`/
    /// `instructions` — every time a `def`/`lambda` inside a loop body
    /// executes. `RefCell` since `CodeObject` is normally shared via `Rc`
    /// (immutable from the outside) but this cache is populated lazily
    /// as a pure implementation detail, not user-visible state — every
    /// entry is a deterministic function of the corresponding `consts[i]`,
    /// so sharing cache hits across any `Rc`-cloned handle to the same
    /// `CodeObject` is always correct, never stale (unlike the `Frame`-level
    /// `LOAD_ATTR`/`LOAD_GLOBAL` inline caches, which read MUTABLE state —
    /// instance/class attributes, globals — and were found this same
    /// session to need real invalidation before they can safely cache).
    pub const_cache: RefCell<Vec<Option<PyObjectRef>>>,
    /// Line number for each instruction (0 = unknown).
    /// Same length as `instructions`; stored separately so Instr stays 8 bytes.
    pub line_numbers: Vec<u32>,
}

/// Structural equality for `CodeObject` — real CPython code objects compare
/// equal when their compiled contents match (test_codeop asserts
/// `compile_command(src) == compile(src, ...)`). Every field is compared
/// EXCEPT `const_cache`, a lazily-populated per-const parse cache that is a
/// pure implementation detail (identical code compiled twice has different
/// caches but must compare equal).
impl PartialEq for CodeObject {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.arg_count == other.arg_count
            && self.posonlyarg_count == other.posonlyarg_count
            && self.kwonlyarg_count == other.kwonlyarg_count
            && self.nlocals == other.nlocals
            && self.instructions == other.instructions
            && self.consts == other.consts
            && self.names == other.names
            && self.varnames == other.varnames
            && self.freevars == other.freevars
            && self.cellvars == other.cellvars
            && self.filename == other.filename
            && self.first_lineno == other.first_lineno
            && self.flags == other.flags
            && self.vararg_name == other.vararg_name
            && self.kwarg_name == other.kwarg_name
            && self.num_defaults == other.num_defaults
            && self.kwonly_defaults_mask == other.kwonly_defaults_mask
            && self.line_numbers == other.line_numbers
    }
}

impl CodeObject {
    pub fn new(name: &str) -> Self {
        CodeObject {
            name: crate::interner::intern(name),
            arg_count: 0,
            posonlyarg_count: 0,
            kwonlyarg_count: 0,
            nlocals: 0,
            instructions: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            varnames: Vec::new(),
            freevars: Box::new(Vec::new()),
            cellvars: Box::new(Vec::new()),
            filename: crate::interner::intern("<unknown>"),
            first_lineno: 1,
            flags: 0,
            vararg_name: None,
            kwarg_name: None,
            num_defaults: 0,
            kwonly_defaults_mask: Box::new(Vec::new()),
            const_cache: RefCell::new(Vec::new()),
            line_numbers: Vec::new(),
        }
    }

    /// Look up the line number for a given instruction index.
    pub fn line_number(&self, idx: usize) -> usize {
        self.line_numbers
            .get(idx)
            .copied()
            .filter(|&ln| ln != 0)
            .map(|ln| ln as usize)
            .unwrap_or(self.first_lineno)
    }

    /// Serialize this CodeObject to a byte vector.
    /// The format is:
    ///   name: str
    ///   arg_count: u32
    ///   kwonlyarg_count: u32
    ///   nlocals: u32
    ///   instructions: Instr[]
    ///   consts: ConstValue[]
    ///   names: str[]
    ///   varnames: str[]
    ///   freevars: str[]
    ///   cellvars: str[]
    ///   filename: str
    ///   first_lineno: u32
    ///   flags: u16
    ///   vararg_name: Option<str>
    ///   kwarg_name: Option<str>
    ///   num_defaults: u32
    ///
    /// Strings are length-prefixed: u16 length + UTF-8 bytes.
    /// Vectors are length-prefixed: u32 count.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        write_str(&mut buf, crate::interner::lookup_str(self.name));
        write_u32(&mut buf, self.arg_count as u32);
        write_u32(&mut buf, self.posonlyarg_count as u32);
        write_u32(&mut buf, self.kwonlyarg_count as u32);
        write_u32(&mut buf, self.nlocals as u32);

        // Instructions (no line_no — stored separately as line_numbers)
        write_u32(&mut buf, self.instructions.len() as u32);
        for instr in &self.instructions {
            write_u16(&mut buf, instr.op as u16);
            write_u32(&mut buf, instr.arg);
        }
        // Line numbers (same length as instructions)
        write_u32(&mut buf, self.line_numbers.len() as u32);
        for &ln in &self.line_numbers {
            write_u32(&mut buf, ln);
        }

        // Constants
        write_u32(&mut buf, self.consts.len() as u32);
        for c in &self.consts {
            write_const_value(&mut buf, c);
        }

        // String vectors
        write_name_vec(&mut buf, &self.names);
        write_name_vec(&mut buf, &self.varnames);
        write_str_vec(&mut buf, &self.freevars);
        write_str_vec(&mut buf, &self.cellvars);

        // Remaining fields
        write_str(&mut buf, crate::interner::lookup_str(self.filename));
        write_u32(&mut buf, self.first_lineno as u32);
        write_u16(&mut buf, self.flags);

        // vararg_name
        match &self.vararg_name {
            Some(s) => {
                write_u8(&mut buf, 1);
                write_str(&mut buf, s);
            }
            None => {
                write_u8(&mut buf, 0);
            }
        }

        // kwarg_name
        match &self.kwarg_name {
            Some(s) => {
                write_u8(&mut buf, 1);
                write_str(&mut buf, s);
            }
            None => {
                write_u8(&mut buf, 0);
            }
        }

        write_u32(&mut buf, self.num_defaults as u32);

        write_u32(&mut buf, self.kwonly_defaults_mask.len() as u32);
        for &b in self.kwonly_defaults_mask.iter() {
            write_u8(&mut buf, if b { 1 } else { 0 });
        }

        buf
    }

    /// Deserialize a CodeObject from a byte slice.
    /// Returns the CodeObject and the number of bytes consumed.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let mut pos: usize = 0;

        let name = crate::interner::intern(&read_str(data, &mut pos)?);
        let arg_count = read_u32(data, &mut pos)? as usize;
        let posonlyarg_count = read_u32(data, &mut pos)? as usize;
        let kwonlyarg_count = read_u32(data, &mut pos)? as usize;
        let nlocals = read_u32(data, &mut pos)? as usize;

        // Instructions (no embedded line_no — stored separately)
        let instr_count = read_u32(data, &mut pos)? as usize;
        let mut instructions = Vec::with_capacity(instr_count);
        for _ in 0..instr_count {
            let op_val = read_u16(data, &mut pos)?;
            let op =
                Opcode::from_u16(op_val).ok_or_else(|| format!("Unknown opcode: {}", op_val))?;
            let arg = read_u32(data, &mut pos)?;
            instructions.push(Instr { op, arg });
        }
        // Line numbers (same length as instructions)
        let ln_count = read_u32(data, &mut pos)? as usize;
        let mut line_numbers = Vec::with_capacity(ln_count);
        for _ in 0..ln_count {
            line_numbers.push(read_u32(data, &mut pos)?);
        }

        // Constants
        let const_count = read_u32(data, &mut pos)? as usize;
        let mut consts = Vec::with_capacity(const_count);
        for _ in 0..const_count {
            consts.push(read_const_value(data, &mut pos)?);
        }

        // String vectors
        let names = read_name_vec(data, &mut pos)?;
        let varnames = read_name_vec(data, &mut pos)?;
        let freevars = Box::new(read_str_vec(data, &mut pos)?);
        let cellvars = Box::new(read_str_vec(data, &mut pos)?);

        // Remaining fields
        let filename = crate::interner::intern(&read_str(data, &mut pos)?);
        let first_lineno = read_u32(data, &mut pos)? as usize;
        let flags = read_u16(data, &mut pos)?;

        // vararg_name
        let vararg_name = if read_u8(data, &mut pos)? != 0 {
            Some(Box::new(read_str(data, &mut pos)?))
        } else {
            None
        };

        // kwarg_name
        let kwarg_name = if read_u8(data, &mut pos)? != 0 {
            Some(Box::new(read_str(data, &mut pos)?))
        } else {
            None
        };

        let num_defaults = read_u32(data, &mut pos)? as usize;

        let kwonly_mask_count = read_u32(data, &mut pos)? as usize;
        let mut kwonly_defaults_mask = Box::new(Vec::with_capacity(kwonly_mask_count));
        for _ in 0..kwonly_mask_count {
            kwonly_defaults_mask.push(read_u8(data, &mut pos)? != 0);
        }

        Ok(CodeObject {
            name,
            arg_count,
            posonlyarg_count,
            kwonlyarg_count,
            nlocals,
            instructions,
            consts,
            names,
            varnames,
            freevars,
            cellvars,
            filename,
            first_lineno,
            flags,
            vararg_name,
            kwarg_name,
            num_defaults,
            kwonly_defaults_mask,
            const_cache: RefCell::new(Vec::new()),
            line_numbers,
        })
    }
}

// ── Serialization helpers ──────────────────────────────────────────

fn write_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
}

fn write_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_str(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    if bytes.len() > 65535 {
        // Truncate gracefully for safety; realistically code objects won't have
        // strings > 64 KiB in their metadata fields.
        write_u16(buf, 65535);
        buf.extend_from_slice(&bytes[..65535]);
    } else {
        write_u16(buf, bytes.len() as u16);
        buf.extend_from_slice(bytes);
    }
}

fn write_str_vec(buf: &mut Vec<u8>, vec: &[String]) {
    write_u32(buf, vec.len() as u32);
    for s in vec {
        write_str(buf, s);
    }
}

fn write_name_vec(buf: &mut Vec<u8>, vec: &[crate::interner::StrId]) {
    write_u32(buf, vec.len() as u32);
    for &id in vec {
        write_str(buf, crate::interner::lookup_str(id));
    }
}

fn read_name_vec(data: &[u8], pos: &mut usize) -> Result<Vec<crate::interner::StrId>, String> {
    let len = read_u32(data, pos)? as usize;
    let mut vec = Vec::with_capacity(len);
    for _ in 0..len {
        let s = read_str(data, pos)?;
        vec.push(crate::interner::intern(&s));
    }
    Ok(vec)
}

fn write_const_value(buf: &mut Vec<u8>, cv: &ConstValue) {
    match cv {
        ConstValue::None => {
            write_u8(buf, 0);
        }
        ConstValue::Bool(v) => {
            write_u8(buf, 1);
            write_u8(buf, if *v { 1 } else { 0 });
        }
        ConstValue::Int(s) => {
            write_u8(buf, 2);
            write_str(buf, s);
        }
        ConstValue::Float(s) => {
            write_u8(buf, 3);
            write_str(buf, s);
        }
        ConstValue::Complex { real, imag } => {
            write_u8(buf, 4);
            write_str(buf, real);
            write_str(buf, imag);
        }
        ConstValue::String(s) => {
            write_u8(buf, 5);
            write_str(buf, s);
        }
        ConstValue::Bytes(b) => {
            write_u8(buf, 6);
            write_u32(buf, b.len() as u32);
            buf.extend_from_slice(b);
        }
        ConstValue::Code(code) => {
            write_u8(buf, 7);
            let code_bytes = code.to_bytes();
            write_u32(buf, code_bytes.len() as u32);
            buf.extend_from_slice(&code_bytes);
        }
        ConstValue::Tuple(items) => {
            write_u8(buf, 8);
            write_str_vec(buf, items);
        }
    }
}

// ── Deserialization helpers ────────────────────────────────────────

fn read_u8(data: &[u8], pos: &mut usize) -> Result<u8, String> {
    if *pos + 1 > data.len() {
        return Err("Unexpected end of data (u8)".to_string());
    }
    let v = data[*pos];
    *pos += 1;
    Ok(v)
}

fn read_u16(data: &[u8], pos: &mut usize) -> Result<u16, String> {
    if *pos + 2 > data.len() {
        return Err("Unexpected end of data (u16)".to_string());
    }
    let bytes: [u8; 2] = [data[*pos], data[*pos + 1]];
    *pos += 2;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, String> {
    if *pos + 4 > data.len() {
        return Err("Unexpected end of data (u32)".to_string());
    }
    let bytes: [u8; 4] = [data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]];
    *pos += 4;
    Ok(u32::from_le_bytes(bytes))
}

fn read_str(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let len = read_u16(data, &mut *pos)? as usize;
    if *pos + len > data.len() {
        return Err("Unexpected end of data (str)".to_string());
    }
    let s = std::str::from_utf8(&data[*pos..*pos + len])
        .map_err(|e| format!("Invalid UTF-8: {}", e))?;
    *pos += len;
    Ok(s.to_string())
}

fn read_str_vec(data: &[u8], pos: &mut usize) -> Result<Vec<String>, String> {
    let count = read_u32(data, pos)? as usize;
    let mut vec = Vec::with_capacity(count);
    for _ in 0..count {
        vec.push(read_str(data, pos)?);
    }
    Ok(vec)
}

fn read_const_value(data: &[u8], pos: &mut usize) -> Result<ConstValue, String> {
    let tag = read_u8(data, pos)?;
    match tag {
        0 => Ok(ConstValue::None),
        1 => {
            let v = read_u8(data, pos)? != 0;
            Ok(ConstValue::Bool(v))
        }
        2 => {
            let s = read_str(data, pos)?;
            Ok(ConstValue::Int(s))
        }
        3 => {
            let s = read_str(data, pos)?;
            Ok(ConstValue::Float(s))
        }
        4 => {
            let real = read_str(data, pos)?;
            let imag = read_str(data, pos)?;
            Ok(ConstValue::Complex { real, imag })
        }
        5 => {
            let s = read_str(data, pos)?;
            Ok(ConstValue::String(s))
        }
        6 => {
            let len = read_u32(data, pos)? as usize;
            if *pos + len > data.len() {
                return Err("Unexpected end of data (Bytes)".to_string());
            }
            let v = data[*pos..*pos + len].to_vec();
            *pos += len;
            Ok(ConstValue::Bytes(v))
        }
        7 => {
            let code_len = read_u32(data, pos)? as usize;
            let code = CodeObject::from_bytes(&data[*pos..*pos + code_len])?;
            *pos += code_len;
            Ok(ConstValue::Code(Box::new(code)))
        }
        8 => {
            let items = read_str_vec(data, pos)?;
            Ok(ConstValue::Tuple(items))
        }
        _ => Err(format!("Unknown ConstValue tag: {}", tag)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    None,
    Bool(bool),
    Int(String),
    Float(String),
    Complex { real: String, imag: String },
    String(String),
    Bytes(Vec<u8>),
    Code(Box<CodeObject>),
    Tuple(Vec<String>),
}

impl ConstValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ConstValue::Int(s) => {
                // Parse string representation, handling 0x, 0o, 0b prefixes
                let s = s.trim();
                if let Ok(v) = s.parse::<i64>() {
                    return Some(v);
                }
                // Handle underscores
                let cleaned: String = s.chars().filter(|c| *c != '_').collect();
                if cleaned.starts_with("0x") || cleaned.starts_with("0X") {
                    i64::from_str_radix(&cleaned[2..], 16).ok()
                } else if cleaned.starts_with("0o") || cleaned.starts_with("0O") {
                    i64::from_str_radix(&cleaned[2..], 8).ok()
                } else if cleaned.starts_with("0b") || cleaned.starts_with("0B") {
                    i64::from_str_radix(&cleaned[2..], 2).ok()
                } else {
                    cleaned.parse::<i64>().ok()
                }
            }
            ConstValue::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }
}
