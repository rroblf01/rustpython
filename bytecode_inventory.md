# RustPython Bytecode Opcode Inventory

**Generated from:**
- `src/bytecode.rs` — Opcode enum definitions (105 opcodes + 10 register ops)
- `src/vm.rs` — `execute_instruction()` handler (lines 722–2417)
- `src/compiler.rs` — opcodes actually emitted by the compiler

---

## ✅ 1. FULLY IMPLEMENTED — Defined AND Handled in VM (95 opcodes)

| # | Opcode | Enum Value | VM Handler | Compiler Emits |
|---|--------|-----------|------------|----------------|
| 1 | `BEFORE_ASYNC_WITH` | 0 | line 2011 | ✓ |
| 2 | `CHECK_EXC_MATCH` | 6 | line 2021 | ✓ |
| 3 | `CLEANUP_THROW` | 7 | line 2377 | — |
| 4 | `END_FOR` | 9 | line 2006 | ✓ |
| 5 | `END_SEND` | 10 | line 2371 | — |
| 6 | `FORMAT_SIMPLE` | 12 | line 2199 | — |
| 7 | `FORMAT_WITH_SPEC` | 13 | line 2204 | — |
| 8 | `GET_AITER` | 14 | line 1987 | ✓ |
| 9 | `GET_ANEXT` | 15 | line 1997 | ✓ |
| 10 | `GET_AWAITABLE` | 66 | line 2326 | — |
| 11 | `GET_ITER` | 67 | line 1446 | ✓ |
| 12 | `CONVERT_VALUE` | 56 | line 2211 | — |
| 13 | `DUP_TOP` | 26 | line 986 | ✓ |
| 14 | `POP_ITER` | 28 | line 2241 | — |
| 15 | `POP_TOP` | 29 | line 982 | ✓ |
| 16 | `PUSH_EXC_INFO` | 30 | line 1966 | ✓ |
| 17 | `PUSH_NULL` | 31 | line 1150 | — |
| 18 | `RETURN_GENERATOR` | 32 | line 2309 | — |
| 19 | `RETURN_VALUE` | 33 | line 1005 | ✓ |
| 20 | `SETUP_ANNOTATIONS` | 34 | line 2239 (no-op) | — |
| 21 | `STORE_SUBSCR` | 36 | line 1791 | ✓ |
| 22 | `UNARY_INVERT` | 38 | line 1383 | — |
| 23 | `UNARY_NEGATIVE` | 39 | line 1372 | — |
| 24 | `UNARY_NOT` | 40 | line 1378 | — |
| 25 | `BINARY_OP` | 42 | line 1311 | ✓ |
| 26 | `BUILD_LIST` | 44 | line 1255 | — |
| 27 | `BUILD_MAP` | 45 | line 1275 | — |
| 28 | `BUILD_SET` | 46 | line 1279 | — |
| 29 | `BUILD_SLICE` | 47 | line 1299 | — |
| 30 | `BUILD_STRING` | 48 | line 1289 | — |
| 31 | `BUILD_TUPLE` | 49 | line 1265 | ✓ |
| 32 | `CALL` | 50 | line 1154 | ✓ |
| 33 | `COMPARE_OP` | 54 | line 1346 | ✓ |
| 34 | `CONTAINS_OP` | 55 | line 1363 | — |
| 35 | `COPY` | 57 | line 991 | ✓ |
| 36 | `COPY_FREE_VARS` | 58 | line 1191 | — |
| 37 | `DELETE_FAST` | 60 | line 970 | ✓ |
| 38 | `DELETE_NAME` | 61 | line 976 | ✓ |
| 39 | `FOR_ITER` | 65 | line 1517 | ✓ |
| 40 | `IMPORT_FROM` | 68 | line 2148 | ✓ |
| 41 | `IMPORT_NAME` | 69 | line 2123 | ✓ |
| 42 | `IS_OP` | 70 | line 1354 | — |
| 43 | `JUMP_BACKWARD` | 71 | line 1395 | ✓ |
| 44 | `JUMP_FORWARD` | 73 | line 1395 | ✓ |
| 45 | `LIST_APPEND` | 74 | line 1822 | — |
| 46 | `LOAD_ATTR` | 76 | line 1603 | ✓ |
| 47 | `LOAD_BUILD_CLASS` | 19 | line 2168 | ✓ |
| 48 | `LOAD_CLOSURE` | 261 | line 2172 | — |
| 49 | `LOAD_CONST` | 78 | line 752 | ✓ |
| 50 | `LOAD_DEREF` | 79 | line 883 | — |
| 51 | `LOAD_FAST` | 80 | line 822 | ✓ |
| 52 | `LOAD_GLOBAL` | 88 | line 848 | ✓ |
| 53 | `LOAD_LOCALS` | 20 | line 2235 | — |
| 54 | `LOAD_NAME` | 89 | line 793 | ✓ |
| 55 | `MAKE_CELL` | 93 | line 1181 | — |
| 56 | `MAKE_FUNCTION` | 21 | line 1201 | — |
| 57 | `MAP_ADD` | 94 | line 1844 | — |
| 58 | `NOP` | 25 | line 750 | ✓ |
| 59 | `POP_BLOCK` | 262 | line 1959 | ✓ |
| 60 | `POP_EXCEPT` | 27 | line 1976 | ✓ |
| 61 | `POP_JUMP_IF_FALSE` | 96 | line 1412 | ✓ |
| 62 | `POP_JUMP_IF_NONE` | 97 | line 1426 | — |
| 63 | `POP_JUMP_IF_NOT_NONE` | 98 | line 1436 | — |
| 64 | `POP_JUMP_IF_TRUE` | 99 | line 1419 | — |
| 65 | `RAISE_VARARGS` | 100 | line 2052 | ✓ |
| 66 | `RERAISE` | 101 | line 2037 | ✓ |
| 67 | `SEND` | 102 | line 2335 | — |
| 68 | `SET_ADD` | 103 | line 1833 | — |
| 69 | `SETUP_CLEANUP` | 263 | line 1949 | — |
| 70 | `SETUP_FINALLY` | 264 | line 1939 | ✓ |
| 71 | `SETUP_WITH` | 265 | line 2245 | ✓ |
| 72 | `STORE_ATTR` | 106 | line 1736 | ✓ |
| 73 | `STORE_DEREF` | 107 | line 932 | — |
| 74 | `STORE_FAST` | 108 | line 835 | ✓ |
| 75 | `STORE_GLOBAL` | 111 | line 874 | — |
| 76 | `STORE_NAME` | 112 | line 813 | ✓ |
| 77 | `SWAP` | 113 | line 997 | ✓ |
| 78 | `UNPACK_EX` | 114 | line 1899 | — |
| 79 | `UNPACK_SEQUENCE` | 115 | line 1877 | — |
| 80 | `WITH_EXIT` | 266 | line 2277 | ✓ |
| 81 | `YIELD_VALUE` | 116 | line 2302 | — |
| 82 | `DELETE_ATTR` | 109 | line 1798 | ✓ |
| 83 | `DICT_MERGE` | 202 | line 1856 | — |
| 84 | `ELSE` | 118 | line 2383 (no-op) | ✓ |
| 85 | `END_FINALLY` | 117 | line 2389 | — |
| 86 | `JUMP` (absolute) | 257 | line 1395 | ✓ |
| 87 | `POP_EXCEPT_AND_EXECUTE_FINALLY` | 119 | line 2407 | — |
| 88 | `REG_MOV` | 0xC0 | line 1011 | — |
| 89 | `REG_LOAD_CONST` | 0xC1 | line 1024 | — |
| 90 | `REG_LOAD_FAST` | 0xC2 | line 1053 | — |
| 91 | `REG_STORE_FAST` | 0xC3 | line 1062 | — |
| 92 | `REG_BINARY_OP` | 0xC4 | line 1075 | — |
| 93 | `REG_LOAD_GLOBAL` | 0xC5 | line 1104 | — |
| 94 | `REG_RETURN` | 0xC7 | line 1129 | — |
| 95 | `REG_BUILD_LIST` | 0xC9 | line 1135 | — |

---

## ⚠️ 2. DEFINED IN Opcode ENUM — BUT **NOT HANDLED** in VM (10 opcodes)

These are defined in `bytecode.rs` with opcode values and `needs_arg` entries, but the `execute_instruction` match block has **no arm** for them. They'll hit the catch-all `_ =>` and return "unimplemented opcode".

| # | Opcode | Enum Value | Notes | Compiler Emits? |
|---|--------|-----------|-------|----------------|
| 1 | `DELETE_SUBSCR` | 110 | 🔴 **EMITTED by compiler** (line 1066)! Runtime will crash with "unimplemented opcode: DELETE_SUBSCR" if `del obj[key]` is used. | ✓ |
| 2 | `CALL_FUNCTION_EX` | 4 | `*args` / `**kwargs` call unpacking — needed for `func(*args)` and `func(**kwargs)` | — |
| 3 | `CALL_KW` | 53 | Keyword-only argument calls — used by CPython for `func(a=1, b=2)` calls | — |
| 4 | `EXTENDED_ARG` | 64 | 32-bit argument prefix — needed when any arg exceeds 255 | — |
| 5 | `LIST_EXTEND` | 75 | Used by list comprehensions with `+=` | — |
| 6 | `LOAD_FROM_DICT_OR_GLOBALS` | 87 | Used by class bodies to resolve names — partial alternative already done inline in LOAD_NAME via `module_globals` | — |
| 7 | `SET_FUNCTION_ATTRIBUTE` | 104 | Sets `__defaults__`, `__kwdefaults__`, `__annotations__` on functions | — |
| 8 | `SET_UPDATE` | 105 | Used by set comprehensions with `\|=` | — |
| 9 | `RESUME` | 128 | Generator/coroutine resume — needed for proper yield-from and async support | — |
| 10 | `REG_CALL` | 0xC6 | Register-based call instruction | — |
| 11 | `REG_JUMP_IF_FALSE` | 0xC8 | Register-based conditional jump | — |

**DELETE_SUBSCR is the most critical:** the compiler emits it for `del obj[key]`, but the VM has no handler — it will crash at runtime.

---

## 🟡 3. DEFINED BUT WITHOUT COMPILER OR VM SUPPORT (Not great — dead enum entries)

These are in the Opcode enum but **never emitted by the compiler** and **not handled by the VM**:

| # | Opcode | Enum Value | Notes |
|---|--------|-----------|-------|
| 1 | `GET_LEN` | 16 | CPython 3.12+ pattern matching support |
| 2 | `MATCH_MAPPING` | 23 | Pattern matching — `match x: case {**rest}:` |
| 3 | `MATCH_SEQUENCE` | 24 | Pattern matching — `match x: case [a, b]:` |
| 4 | `MATCH_KEYS` | 22 | Pattern matching — key-pattern matching |
| 5 | `CALL_INTRINSIC_1` | 51 | CPython 3.13+ intrinsic calls (unary) |
| 6 | `CALL_INTRINSIC_2` | 52 | CPython 3.13+ intrinsic calls (binary) |
| 7 | `UNPACK_SEQUENCE_TWO_TUPLE` | 218 | Optimized 2-tuple unpacking (CPython 3.12+) |

These are technically dead code in this RustPython version — they exist in the enum but nothing generates or handles them.

---

## 🟠 4. HANDLED IN `try_exec_simple()` (inline fast-path, not main VM dispatch)

These are duplicated in the simple-execution path (lines 626–689) but are also handled in the main `execute_instruction`:

- `LOAD_FAST`, `STORE_FAST`, `LOAD_CONST`, `BINARY_OP`, `COMPARE_OP`, `POP_JUMP_IF_FALSE`, `JUMP_FORWARD`, `JUMP_BACKWARD`, `RETURN_VALUE`

These work fine — they have both fast-path and full-path handlers.

---

## 🚩 5. CPython 3.14 OPCODES THAT ARE MISSING ENTIRELY

Comparing against CPython 3.13/3.14's actual instruction set, these opcodes are **completely absent** (not even defined in the enum):

| CPython Opcode | Approx Value | Why Needed |
|---------------|-------------|-----------|
| **INTERPRETER_EXIT** | — | Replaces RETURN_VALUE in optimized code objects (3.13+) |
| **INSTRUMENTED_*** | various | Debug/profiling instrumentation (3.12+) |
| **JUMP_BACKWARD_NO_INTERRUPT** | — | Non-interruptible backward jump |
| **LOAD_FAST_AND_CLEAR** | — | Fast local load+clear for `try: except:` patterns (3.12+) |
| **LOAD_FAST_CHECK** | — | LOAD_FAST with unbound-local check (3.12+) |
| **LOAD_FAST_LOAD_FAST** | — | Combined LOAD_FAST × 2 (3.13+) |
| **LOAD_CONST__LOAD_FAST** | — | Combined LOAD_CONST + LOAD_FAST (3.13+) |
| **STORE_FAST__LOAD_FAST** | — | Combined STORE_FAST + LOAD_FAST (3.13+) |
| **LOAD_FAST__STORE_FAST** | — | Combined LOAD_FAST + STORE_FAST (3.13+) |
| **LOAD_GLOBAL_MODULE** | — | Specialized LOAD_GLOBAL for module-level (3.12+) |
| **LOAD_GLOBAL_BUILTIN** | — | Specialized LOAD_GLOBAL for builtins (3.12+) |
| **BINARY_SLICE** | — | Optimized slice operations (3.12+) |
| **STORE_SLICE** | — | Optimized slice store operations (3.12+) |
| **TO_BOOL** | — | Truthiness conversion (3.13+) |
| **PUSH_EXC_HANDLER** | — | New exception handling model in 3.13+ |
| **POP_EXCEPT_AND_RERAISE** | — | Simplified except handler ending (3.13+) |
| **ASYNC_GENERATOR_WRAP** | — | Async generator for `for` target |
| **CALL_ALLOC_AND_ENTER_INIT** | — | Optimized class instantiation (3.13+) |
| **LOAD_SUPER_ATTR** | — | `super()` attribute access optimization (3.13+) |
| **MAKE_CELL__STORE_FAST** | — | Combined MAKE_CELL + STORE_FAST (3.13+) |
| **MAKE_CELL__LOAD_FAST** | — | Combined MAKE_CELL + LOAD_FAST (3.13+) |
| **STORE_ATTR_INSTANCE_VALUE** | — | Specialized STORE_ATTR (3.13+) |
| **LOAD_ATTR_INSTANCE_VALUE** | — | Specialized LOAD_ATTR (3.13+) |
| **LOAD_ATTR_WITH_HINT** | — | LOAD_ATTR with type version hint (3.13+) |
| **LOAD_ATTR_MODULE** | — | Specialized module attribute load (3.13+) |
| **LOAD_ATTR_SLOT** | — | Specialized slot attribute load (3.13+) |
| **LOAD_ATTR_METHOD_LAZY_DICT** | — | Optimized method lookup (3.13+) |
| **LOAD_ATTR_METHOD_NO_DICT** | — | Optimized method lookup (3.13+) |
| **LOAD_ATTR_PROPERTY** | — | Optimized property access (3.13+) |
| **LOAD_ATTR_GETATTRIBUTE_OVERRIDDEN** | — | Overridden __getattr__ path (3.13+) |

**Note:** Many CPython 3.13+ "superinstructions" (combined ops like `LOAD_FAST_LOAD_FAST`) are optimization-only; a VM can be compatible without them if it handles the individual instructions. The table above highlights the **semantically required** new opcodes.

---

## 🔍 SUMMARY

| Category | Count |
|----------|-------|
| ✅ **Fully implemented** (defined + VM handler) | **95** (including 8 register ops) |
| ⚠️ **Defined but NOT handled** (VM will error) | **11** (1 is **fatal**: `DELETE_SUBSCR` is emitted by compiler!) |
| 🟡 **Dead enum entries** (defined, no consumer) | **7** (pattern matching intrinsics, etc.) |
| 🚩 **Missing entirely** for CPython 3.14 compat | **~30+** (new 3.12–3.14 opcodes) |

### 🔴 Critical Bug Found
`DELETE_SUBSCR` (opcode 110) is **emitted by the compiler** for `del obj[key]` expressions (line 1066 in `compiler.rs`), but has **no handler** in `execute_instruction()`. Attempting `del dict_var["key"]` or `del list_var[0]` will crash with:
```
RuntimeError: unimplemented opcode: DELETE_SUBSCR
```
