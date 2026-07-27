# Phase 3: Remaining Optimization Items

Current gap vs CPython: ~10× speed, +22% RAM

---

## Item A: Computed Goto Dispatch (Dispatch Table)

**Goal:** Replace the 106-arm `match op { ... }` in `execute_instruction` (vm.rs:2075-5107) with an O(1) dispatch table.

**Estimated speedup:** 1.5-2× on dispatch-heavy code, 0.5-1× overall.

### Problem

The main dispatch loop uses `match op`:
```rust
match op {
    Opcode::LOAD_FAST => { ... },
    Opcode::LOAD_CONST => { ... },
    ... // ~106 arms spanning 3000 lines
}
```

LLVM compiles this to a binary search tree (O(log n) ≈ 7-10 comparisons) because the Opcode values are sparse (4 to 116 with gaps). CPython uses computed goto (`goto *opcode_table[op]`), which is O(1).

### Solution: Categorize + Dispatch Table

**Strategy:** Two-level dispatch. First level categorizes the opcode into a group (5-15 members each, dense), second level dispatches within the group (dense → jump table).

**Implementation steps:**

1. **Define opcode groups** — Add a method `Opcode::group()`:
```rust
impl Opcode {
    pub fn group(self) -> OpGroup {
        match self {
            Opcode::LOAD_FAST | Opcode::LOAD_CONST | Opcode::LOAD_GLOBAL | ... => OpGroup::Load,
            Opcode::STORE_FAST | Opcode::STORE_NAME | Opcode::STORE_GLOBAL | ... => OpGroup::Store,
            Opcode::BINARY_OP | Opcode::UNARY_NEGATIVE | ... => OpGroup::Arith,
            Opcode::CALL | Opcode::CALL_FUNCTION_EX | Opcode::CALL_KW => OpGroup::Call,
            Opcode::JUMP_FORWARD | Opcode::JUMP_BACKWARD | Opcode::POP_JUMP_IF_FALSE | ... => OpGroup::Jmp,
            Opcode::RETURN_VALUE | Opcode::YIELD_VALUE | Opcode::RAISE_VARARGS => OpGroup::Ret,
            Opcode::BUILD_LIST | Opcode::BUILD_TUPLE | Opcode::BUILD_MAP | ... => OpGroup::Build,
            _ => OpGroup::Other,
        }
    }
}
```

2. **Extract handler functions** — Each match arm becomes a standalone handler:
```rust
type Handler = fn(&mut ExecuteCtx) -> ControlFlow;

fn handle_load_fast(ctx: &mut ExecuteCtx) -> ControlFlow { ... }
fn handle_load_const(ctx: &mut ExecuteCtx) -> ControlFlow { ... }
// ... 106 handlers
```

3. **Replace dispatch loop**:
```rust
fn execute_inner(&mut self, fi: usize) -> PyResult<PyObjectRef> {
    let mut ctx = ExecuteCtx { vm: self, fi };
    loop {
        let instr = &ctx.vm.frames[ctx.fi].code.instructions[ctx.vm.frames[ctx.fi].ip];
        ctx.vm.frames[ctx.fi].ip += 1;
        let op = instr.op;
        let arg = instr.arg;
        
        match op.group() {
            OpGroup::Load => match op {
                Opcode::LOAD_FAST => handle_load_fast(&mut ctx, arg),
                Opcode::LOAD_CONST => handle_load_const(&mut ctx, arg, instr),
                // ...
            },
            OpGroup::Store => match op { ... },
            // ...
        }
    }
}
```

### Files to modify:
- `src/vm.rs` — Add `ExecuteCtx` struct, extract 106 handlers, replace dispatch loop (~3000 lines changed)
- `src/bytecode.rs` — Add `Opcode::group()` method (~40 lines)

### Verification:
- `cargo build` (0 errors)
- `make test` passes
- `RPY_DEBUG_SIZES` works (no functional changes)

### Risk:
- High — 3000 lines of code movement. Easy to introduce subtle bugs.
- Mitigation: extract handlers one group at a time, test after each group.

---

## Item B: HashMap<StrId> for Frame globals/builtins

**Goal:** Change `HashMap<String, PyObjectRef>` → `HashMap<StrId, PyObjectRef>` for Frame-level globals and builtins.

**Estimated speedup:** 5-15% for LOAD_GLOBAL-heavy code. **RAM savings:** 5-10%.

### Problem

Frame.globals and Frame.builtins use `HashMap<String, PyObjectRef>`. Each key is a `String` (24 bytes + heap allocation). The LOAD_GLOBAL handler calls:
```rust
let name_str = lookup_str(name_id);  // &'static str from interner
globals.borrow().get(name_str)       // str → String compare during HashMap lookup
```

With `HashMap<StrId>`, the lookup becomes:
```rust
globals.borrow().get(&name_id)       // u32 → u32 compare
```

This avoids:
1. The `lookup_str` call (GLOBAL_INTERNER.with() + Vec index)
2. The string comparison during HashMap::get (u32 compare instead of str compare)
3. String allocation on insert (just the u32)

### Implementation steps (file by file):

#### Step 1: vm.rs — Frame struct + methods

Change field types:
```rust
pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
pub builtins: Rc<HashMap<StrId, PyObjectRef>>,
pub module_globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
```

Update `Frame::new()` parameters:
```rust
fn new(
    code: Rc<CodeObject>,
    globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
    builtins: Rc<HashMap<StrId, PyObjectRef>>,
    module_globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
) -> Self
```

#### Step 2: vm.rs — All lookup sites

For each `.get(key)` where key is `&str`, change to `.get(&interned_key)`.

Patterns to fix (~30 sites):
```
globals.get("__name__")          → globals.get(&intern("__name__"))
globals.get(&name_str)           → globals.get(&intern(&name_str)) 
globals.get(name)                → globals.get(&intern(name))
globals.borrow().get(name_str)   → globals.borrow().get(&intern(name_str))
```

For `.insert(key, val)` where key is `String`, change:
```
globals.insert(name.to_string(), val)  → globals.insert(intern(&name), val)
globals.insert(name, val)              → globals.insert(intern(&name), val)
```

#### Step 3: vm.rs — All insertion sites

~10 sites where module globals are populated:
```rust
// In install_source_defined_stdlib:
dict.insert(intern("__name__"), py_str(module_name));

// In import_module_from_file / exec_module_source:
g.borrow_mut().insert(intern("__builtins__"), ...);
```

For `name_order: Option<Rc<RefCell<Vec<String>>>>` — this stays as-is (it tracks insertion order for display, not lookups).

#### Step 4: object.rs — PyFunction.globals + call sites

Change PyFunction:
```rust
pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
```

In `call_bound_method` and `builtin_call` (object.rs), the Function handler creates a frame with the function's globals. Change the frame creation to pass `HashMap<StrId>`.

#### Step 5: main.rs — CLI entry point

The main function creates the initial `globals` dict:
```rust
let globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>> = Rc::new(RefCell::new(HashMap::new()));
globals.borrow_mut().insert(intern("__file__"), py_str(filename));
```

#### Step 6: VirtualMachine struct

The VM has its own `builtins` field:
```rust
pub builtins: Rc<HashMap<StrId, PyObjectRef>>,
```

Update the `create_builtins()` function to use `intern()` for keys.

#### Step 7: sync.rs, modules/*.rs

Any code that accesses `vm.builtins` or `frame.globals` with string keys needs updating.

### Files to modify:
- `src/vm.rs` — ~40 sites
- `src/object.rs` — ~10 sites
- `src/main.rs` — 2-3 sites
- `src/sync.rs` — 1-2 sites
- `src/modules/*.rs` — ~5 sites

### Verification:
- `cargo build` (0 errors after each step)
- `RPY_DEBUG_SIZES=1 ./target/debug/rustpython -c ""` works
- `make test` passes
- `./target/release/rustpython /tmp/bench_speed.py` measurably faster

### Risk:
- Medium — many mechanical changes but each is straightforward.
- The `.get(&intern(...))` pattern adds an `intern()` lookup on EVERY cache miss. This is slower than direct `HashMap<String>::get(&str)`. But cache HITS with StrId are faster.
- Solution: pre-compute `StrId` values for known strings (like `"__name__"`) at init time using `LazyLock`:
```rust
use std::sync::LazyLock;
static NAME_STR: LazyLock<StrId> = LazyLock::new(|| crate::interner::intern("__name__"));
```
But the interner is thread-local, so this won't work with `LazyLock`. 
Alternative: compute on first use and cache in a thread-local:
```rust
thread_local! {
    static NAME_STR: StrId = crate::interner::intern("__name__");
}
```
But you can't initialize `thread_local!` with non-const expressions in stable Rust.

Best approach: just use `intern()` inline. It's a RefCell::borrow_mut + HashMap lookup, which is ~50ns on first call and ~20ns on subsequent calls (when the string is already interned). This is faster than a full string comparison for HashMap::get.

---

## Item C: Extend JIT with MAKE_FUNCTION + more opcodes

**Goal:** Add MAKE_FUNCTION and other missing opcodes to the JIT's supported list so functions using them can be JIT-compiled.

**Estimated speedup:** 1-2× on code with closures/loops (like the benchmark's `make_funcs`).

### Problem

The JIT aborts compilation for the **entire function** if ANY opcode is unsupported (line 884):
```rust
if !supported.contains(&instr.op) {
    eprintln!("JIT: unsupported opcode {:?} in '{}'", instr.op, code.name);
    return None;
}
```

Currently supported: 42 opcodes.
Missing crucial opcodes:
- `MAKE_FUNCTION` (function/closure creation in loops)
- `STORE_NAME` (module-level name binding)
- `DELETE_NAME`, `DELETE_FAST`
- `STORE_DEREF`, `LOAD_DEREF` (for closures/nonlocal)
- `BUILD_CLASS` (class creation)
- `LIST_EXTEND`, `MAP_ADD`, `SET_ADD`, `SET_UPDATE` (comprehensions)
- `CALL_FUNCTION_EX` (star-args calls)
- `BINARY_OP` arg 13 (BINARY_SUBSCR) — item access `a[i]`

### Strategy: Three-tier approach

#### Tier 1: Add runtime-call helpers (highest impact, most achievable)

For complex opcodes like MAKE_FUNCTION, add an `extern "C"` helper (like `jit_call`, `jit_load_attr`) that does the full operation. The JIT emits a call to this helper instead of aborting.

**For MAKE_FUNCTION:**

1. Add `extern "C" fn jit_make_function(stack: *mut PyObjectRef, sp: i64) -> i64` in jit.rs:
   - Reads stack[sp-1] through stack[sp-4] for closure, defaults, code, name
   - Creates PyFunction
   - Writes result to stack[sp-4]
   - Returns new stack pointer (sp - 3)

2. Add `make_function_func` field to `JitCompiler` struct
3. Declare and import the function reference in the Cranelift IR
4. In the opcode compilation match, emit a call to `jit_make_function`

**For BINARY_SUBSCR (BINARY_OP arg=13):**

Add `extern "C" fn jit_subscr(stack: *const PyObjectRef, obj_idx: i64, key_idx: i64) -> PyObjectRef`:
- Reads obj and key from stack
- Calls `py_getitem(&obj, &key)`
- Returns result

#### Tier 2: Add simple inline opcodes

For opcodes that can be compiled to a few Cranelift instructions:
- `STORE_NAME` — already similar to `STORE_FAST` but needs dict access
- `LOAD_DEREF` — similar to `LOAD_FAST`
- `DELETE_NAME`, `DELETE_FAST` — remove from locals

#### Tier 3: Add comprehension opcodes

`LIST_EXTEND`, `MAP_ADD`, `SET_ADD`, `SET_UPDATE` — these are used in comprehensions. They modify lists/dicts/sets in-place. Each would need a runtime helper similar to `jit_build_map`.

### Implementation details for MAKE_FUNCTION

**Step 1: Add the extern C helper:**

```rust
extern "C" fn jit_make_function(stack: *mut PyObjectRef, sp: i64) -> i64 {
    unsafe {
        let closure = (*stack.offset(sp - 1)).clone();
        let defaults = (*stack.offset(sp - 2)).clone();
        let code = (*stack.offset(sp - 3)).clone();
        let name = (*stack.offset(sp - 4)).clone();
        // Create function
        let code_obj: Rc<CodeObject> = ...;
        let func = PyObject::Function(Box::new(PyFunction {
            code: code_obj,
            globals: /* need access to current globals */,
            defaults: ...,
            closure: ...,
            dict: HashMap::new(),
            jit_ptr: Cell::new(0),
            jit_consts: RefCell::new(Vec::new()),
        }));
        *stack.offset(sp - 4) = PyObjectRef::new(func);
        sp - 3  // Pop 3 items (closure+defaults+code), keep name
    }
}
```

**Problem:** The MAKE_FUNCTION helper needs access to the current frame's `globals` to set `__module__`. The JIT-compiled function doesn't pass globals directly. Current approach in the JIT: the first parameter to the compiled function is `fast_locals` ptr. Globals aren't passed.

**Solution:** Pass the frame pointer (or a context struct) as an additional parameter to the compiled function.

Currently, the JIT function signature is:
```rust
fn(fast_locals: *const PyObjectRef, consts: *const PyObjectRef, args_count: i64, nlocals: i64) -> i64
```

I need to add globals (or a frame pointer):
```rust
fn(fast_locals: *const PyObjectRef, consts: *const PyObjectRef, args_count: i64, nlocals: i64, frame_ptr: *const Frame) -> i64
```

But `Frame` contains `Rc`s and other Rust types that are not FFI-safe. Better: pass globals as a raw pointer:
```rust
fn(... , globals: *const RefCell<HashMap<String, PyObjectRef>>) -> i64
```

This requires changes to:
1. The JIT function signature (line 892-895)
2. The call site in vm.rs where JIT is invoked (line 5726-5735)
3. All existing helpers that access the stack/context

This is the most complex part of the change.

### Files to modify:
- `src/jit.rs` — Add helpers, add opcodes to supported list, modify function signature
- `src/vm.rs` — Update JIT call site with new parameters

### Verification:
- `cargo build --features jit` (0 errors)
- `RPY_NO_JIT=1 ./target/release/rustpython /tmp/bench_speed.py` vs `./target/release/rustpython /tmp/bench_speed.py` (JIT should be faster)
- `make test` passes

### Risk:
- High — the Cranelift API has no stability guarantees. Changes might break on Cranelift version updates.
- The extern C helpers have `unsafe` code. Undefined behavior possible if stack offsets are wrong.
- The JIT function signature change affects ALL compiled functions, not just ones using new opcodes.

---

## Recommended Execution Order

```
Week 1:   Item B (HashMap<StrId>) — 30-40 mechanical changes, well-understood.
          Test after each file.
          
Week 2:   Item A (dispatch table) — Extract handlers group by group.
          Test after each group (7-8 groups).
          
Week 3:   Item C (JIT extension) — Most complex. Add one opcode at a time.
          Start with MAKE_FUNCTION, then BINARY_SUBSCR, then comprehension opcodes.
```

**Priority:** B → A → C. Reason: B has the highest certainty of success and measurable impact. A has high impact but higher risk. C has the highest risk but also the highest ceiling for speed improvement on JIT-compilable code.
