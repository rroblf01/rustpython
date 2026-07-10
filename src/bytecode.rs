use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
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
    IS_OP = 70,
    JUMP_BACKWARD = 71,
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
    SEND = 102,
    SET_ADD = 103,
    SET_FUNCTION_ATTRIBUTE = 104,
    SET_UPDATE = 105,
    STORE_ATTR = 106,
    STORE_DEREF = 107,
    STORE_FAST = 108,
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
    UNPACK_SEQUENCE_TWO_TUPLE = 218,
}

impl Opcode {
    pub fn from_u16(n: u16) -> Option<Opcode> {
        use Opcode::*;
        Some(match n {
            42 => BINARY_OP,
            44 => BUILD_LIST,
            45 => BUILD_MAP,
            46 => BUILD_SET,
            47 => BUILD_SLICE,
            48 => BUILD_STRING,
            49 => BUILD_TUPLE,
            50 => CALL,
            54 => COMPARE_OP,
            55 => CONTAINS_OP,
            64 => EXTENDED_ARG,
            65 => FOR_ITER,
            67 => GET_ITER,
            68 => IMPORT_FROM,
            69 => IMPORT_NAME,
            70 => IS_OP,
            71 => JUMP_BACKWARD,
            73 => JUMP_FORWARD,
            74 => LIST_APPEND,
            76 => LOAD_ATTR,
            78 => LOAD_CONST,
            79 => LOAD_DEREF,
            80 => LOAD_FAST,
            88 => LOAD_GLOBAL,
            89 => LOAD_NAME,
            93 => MAKE_CELL,
            96 => POP_JUMP_IF_FALSE,
            97 => POP_JUMP_IF_NONE,
            98 => POP_JUMP_IF_NOT_NONE,
            99 => POP_JUMP_IF_TRUE,
            100 => RAISE_VARARGS,
            // 101 => RERAISE,
            102 => SEND,
            103 => SET_ADD,
            106 => STORE_ATTR,
            107 => STORE_DEREF,
            108 => STORE_FAST,
            111 => STORE_GLOBAL,
            112 => STORE_NAME,
            113 => SWAP,
            115 => UNPACK_SEQUENCE,
            116 => YIELD_VALUE,
            25 => NOP,
            29 => POP_TOP,
            31 => PUSH_NULL,
            33 => RETURN_VALUE,
            38 => UNARY_INVERT,
            39 => UNARY_NEGATIVE,
            40 => UNARY_NOT,
            28 => DUP_TOP,
            0 => STORE_SUBSCR,
            261 => LOAD_CLOSURE,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Instr {
    pub op: Opcode,
    pub arg: u32,
}

impl Instr {
    pub fn new(op: Opcode) -> Self {
        Instr { op, arg: 0 }
    }

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

fn needs_arg(op: Opcode) -> bool {
    use Opcode::*;
    matches!(op,
        BINARY_OP | BUILD_LIST | BUILD_MAP | BUILD_SET | BUILD_SLICE |
        BUILD_STRING | BUILD_TUPLE | CALL | CALL_FUNCTION_EX | CALL_KW |
        COMPARE_OP | CONTAINS_OP | COPY | COPY_FREE_VARS | DELETE_FAST |
        DELETE_NAME | EXTENDED_ARG | FOR_ITER | IMPORT_FROM | IMPORT_NAME |
        IS_OP | JUMP_BACKWARD | JUMP_FORWARD | LIST_APPEND | LIST_EXTEND |
        LOAD_ATTR | LOAD_CONST | LOAD_DEREF | LOAD_FAST | LOAD_FROM_DICT_OR_GLOBALS |
        LOAD_GLOBAL | LOAD_NAME | MAKE_CELL | MAP_ADD | POP_JUMP_IF_FALSE |
        POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE | POP_JUMP_IF_TRUE |
        RAISE_VARARGS | RERAISE | SEND | SET_ADD | SET_FUNCTION_ATTRIBUTE |
        SET_UPDATE | STORE_ATTR | STORE_DEREF | STORE_FAST | STORE_GLOBAL |
        STORE_NAME | SWAP | UNPACK_EX | UNPACK_SEQUENCE | YIELD_VALUE |
        RESUME | JUMP | POP_BLOCK | SETUP_FINALLY | SETUP_CLEANUP | SETUP_WITH |
        MAKE_FUNCTION | LOAD_BUILD_CLASS | PUSH_NULL | RETURN_VALUE | UNARY_NEGATIVE |
        UNARY_NOT | UNARY_INVERT | GET_LEN | MATCH_MAPPING | MATCH_SEQUENCE |
        MATCH_KEYS | CHECK_EXC_MATCH | PUSH_EXC_INFO | END_FOR | GET_AITER |
        GET_ANEXT | GET_AWAITABLE | CLEANUP_THROW | END_SEND | FORMAT_SIMPLE |
        FORMAT_WITH_SPEC | CONVERT_VALUE | LOAD_LOCALS | RETURN_GENERATOR |
        SETUP_ANNOTATIONS | POP_EXCEPT | UNPACK_SEQUENCE_TWO_TUPLE |
        DUP_TOP | STORE_SUBSCR | LOAD_CLOSURE | POP_ITER
    )
}

#[derive(Debug, Clone)]
pub struct CodeObject {
    pub name: String,
    pub arg_count: usize,
    pub nlocals: usize,
    pub instructions: Vec<Instr>,
    pub consts: Vec<ConstValue>,
    pub names: Vec<String>,
    pub varnames: Vec<String>,
    pub freevars: Vec<String>,
    pub cellvars: Vec<String>,
    pub filename: String,
    pub first_lineno: usize,
    pub flags: u16,
}

impl CodeObject {
    pub fn new(name: String) -> Self {
        CodeObject {
            name,
            arg_count: 0,
            nlocals: 0,
            instructions: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            varnames: Vec::new(),
            freevars: Vec::new(),
            cellvars: Vec::new(),
            filename: "<unknown>".to_string(),
            first_lineno: 1,
            flags: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConstValue {
    None,
    Bool(bool),
    Int(String),
    Float(String),
    String(String),
    Code(Box<CodeObject>),
}
