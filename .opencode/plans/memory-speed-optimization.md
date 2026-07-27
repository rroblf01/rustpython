# Memory & Speed Optimization Plan (Priority Queue)

**Current state:** PyObject=112B, PyDict=80B, PySet=56B, Frame=296B, CodeObject=344B

---

## #1: Box Property → `Property(Box<PropertyData>)`

**Impact:** PyObject 112→96 bytes (14% reduction for ALL Python objects)

**Steps:**
1. Add `PropertyData` struct after PyObject enum (object.rs:~1510)
2. Change variant: `Property { getter, setter, deleter, doc }` → `Property(Box<PropertyData>)`
3. Update 26 match arms across 6 files:

### Pattern guide for each case:

| Pattern type | Before | After | Count |
|---|---|---|---|
| Simple ignore | `Property { .. }` | `Property(_)` | 2 |
| Destructure subset | `Property { getter, deleter, doc, .. }` | `Property(ref d)` + `d.getter.clone()` | 4 |
| Destructure all | `Property { getter, setter, deleter, doc }` | `Property(ref d)` + fields from `d` | 2 |
| Conditional Some(g) | `Property { getter: Some(g), .. }` | `Property(ref d)` + `if d.getter.is_some()` | 5 |
| `if let` with Some | `if let Property { setter: Some(s), .. } = &*d` | `if let Property(ref d) = &*d` + nested `d.setter.as_ref()` | 3 |
| Construction | `Property { getter, .. }` | `Property(Box::new(PropertyData { getter, .. }))` | 10 |

### Files to modify:
- `src/object.rs` — 14 sites (definition + 13 match/construction)
- `src/vm.rs` — 5 sites
- `src/modules/files.rs` — 4 sites (all construction)
- `src/modules/misc.rs` — 3 sites (all construction)
- `src/modules/dev.rs` — 1 site (construction)
- `src/cycle_gc.rs` — 1 site

### Verification:
- `cargo build` (0 errors)
- `PY_DEBUG_SIZES=1 ./target/debug/rustpython -c ""` → confirms PyObject: 96 bytes
- `make test` passes

---

## #2: CodeObject — Box Cold Fields

**Impact:** CodeObject 344→~264 bytes

**Steps (one field at a time, rebuild after each):**

1. **Box `freevars`**: `Vec<String>` → `Box<Vec<String>>` (24→8, save 16)
2. **Box `cellvars`**: `Vec<String>` → `Box<Vec<String>>` (24→8, save 16)
3. **Box `vararg_name`**: `Option<String>` → `Option<Box<String>>` (24→8, save 16)
4. **Box `kwarg_name`**: same as #3 (24→8, save 16)
5. **Box `kwonly_defaults_mask`**: `Vec<bool>` → `Box<Vec<bool>>` (24→8, save 16)

### Access pattern changes:
- `code.freevars[i]` → `code.freevars[i]` (auto-deref via Box)
- `code.freevars.len()` → same
- `code.freevars.clone()` → `(*code.freevars).clone()` or `code.freevars.clone()` (auto-deref via Box)
- `code.freevars.push(x)` → `code.freevars.as_mut().push(x)` or use temporary
- `code.vararg_name.as_deref()` → unchanged (Box implements Deref)
- `if let Some(vararg_name) = &code.vararg_name` → unchanged (Box pattern works the same as String for pattern matching)

### Key compilation issues to watch:
- `compiler.rs` assignments: `freevars.push(...)` needs `freevars.as_mut().unwrap().push(...)` or construct elsewhere
- `bytecode.rs` serialization/deserialization: wrap/unwrap in Box
- `object.rs` comparisons: `code.vararg_name == &other.vararg_name` — needs deref adjustment
- `dev.rs`: any code accessing these fields directly

### Verification:
- `cargo build` (0 errors after each field)
- `PY_DEBUG_SIZES=1 ./target/debug/rustpython -c ""` → CodeObject: ~264 bytes
- `make test` passes

---

## #3: CodeObject name/filename → Interned StrId

**Impact:** CodeObject 264→~256 bytes (+ removes heap alloc per unique name/filename)

**Steps:**
1. Change `name: String` → `name: StrId` in CodeObject
2. Change `filename: String` → `filename: StrId` in CodeObject
3. Update `CodeObject::new()` to intern the strings
4. Update all access sites that call `.as_str()` on name/filename
5. In `compiler.rs`, intern before assigning
6. In traceback/error messages, call `lookup_str(name)` to get the string back

### Files to modify:
- `src/bytecode.rs` — struct definition, serialization
- `src/compiler.rs` — construction
- `src/object.rs` — display/errors using code.name
- `src/modules/dev.rs` — traceback introspection

### Verification:
- `cargo build` (0 errors)
- `make test` passes (tracebacks still display correctly)

---

## #4: Remove redundant `PyFunction.name`

**Impact:** -24 bytes per function object

**Steps:**
1. Remove `name: String` from `PyFunction` struct
2. In `MAKE_FUNCTION` (compiler/vm): don't assign `name`
3. In `display()` for Function: use `code.name` instead
4. In `__name__` attribute access: return `code.name` stringified

### Files to modify:
- `src/object.rs` — struct, display, attribute access (~15 sites)

### Verification:
- `cargo build` (0 errors)
- `make test` passes
- `fn.__name__` still works correctly

---

## #5: Enable JIT by Default

**Impact:** Significant speedup for hot loops (5-50x on numeric code)

**Steps:**
1. In `Cargo.toml`, add `jit` to `default` features list
2. Verify `make build` still works (Cranelift adds ~2s to compile time)
3. Run benchmarks: `cd benchmarks && python3 -c "..."` vs system CPython

### Files to modify:
- `Cargo.toml`

### Verification:
- `cargo build --no-default-features` still works (for static builds)
- JIT activates on hot bytecode regions (visible in `JIT:` prefixed debug output)

---

## #6: Compiler Peephole Optimizations

**Impact:** Medium — reduces runtime work for common patterns

**Steps:**
1. **Constant folding** in `compiler.rs`:
   - `LOAD_CONST(int) + LOAD_CONST(int) → LOAD_CONST(int)` for +, -, *, //, %
   - `LOAD_CONST(str) + LOAD_CONST(str) → LOAD_CONST(str)`
   - `LOAD_CONST(float) + LOAD_CONST(float) → LOAD_CONST(float)`
2. **BUILD_STRING of constant strings**: fold into single `LOAD_CONST`
3. **Dead code elimination**: after `RETURN_VALUE` or `RAISE_VARARGS`, mark subsequent instructions as unreachable
4. **Tuple/list of constants**: replace `LOAD_CONST(a), LOAD_CONST(b), BUILD_TUPLE(2)` → `LOAD_CONST((a, b))`

### Files to modify:
- `src/compiler.rs` — add `optimize()` pass in `compile()` or after instruction emission

### Verification:
- `cargo build` (0 errors)
- `make test` passes
- Verify: `dis.dis("x = 1+2")` shows `LOAD_CONST 3` instead of two loads + add
- Verify: `dis.dis("def f(): return 1; x = 2")` shows no dead code

---

## #7: Zero-Allocation Iteration for PyDict/PySet

**Impact:** Reduces allocation pressure on all set/dict operations

**Steps:**
1. Add `iter()` method to PyDict:
   ```rust
   pub fn iter(&self) -> impl Iterator<Item = (&PyObjectRef, &PyObjectRef)> {
       self.entries.iter().filter_map(|e| {
           e.as_ref().map(|(k, v)| (k, v))
       })
   }
   ```
2. Add `iter()` to PySet:
   ```rust
   pub fn iter(&self) -> impl Iterator<Item = &PyObjectRef> {
       self.entries.iter().filter_map(|e| e.as_ref())
   }
   ```
3. Update `to_vec()` to use `iter().cloned().collect()`
4. Update `is_superset()` to iterate without cloning

### Files to modify:
- `src/object.rs` — PyDict and PySet impl blocks

### Verification:
- `cargo build` (0 errors)
- `make test` passes
- Compare: `{1,2,3}.issuperset({1,2})` no longer allocates

---

## #8: Intern Strings for LOAD_ATTR Dispatch

**Impact:** Faster attribute access (string compares → int compares)

**Steps:**
1. Convert the attribute-name lookup table in `get_attribute` / `LOAD_ATTR` handlers to use `StrId` comparisons
2. Intern known attribute names (`"__get__"`, `"__set__"`, `"__delete__"`, `"fget"`, `"fset"`, `"fdel"`, `"__getattribute__"`, `"__getattr__"`) at function scope
3. Compare via `StrId` instead of `String`/`&str`

### Files to modify:
- `src/object.rs` — `get_attribute` and `set_attribute` implementations
- `src/vm.rs` — LOAD_ATTR/STORE_ATTR handlers

### Verification:
- `cargo build` (0 errors)
- `make test` passes

---

## Execution Order & Dependencies

```
#1 (Property boxing)  ──────→  independent, do first
#2 (CodeObject fields)  ────→  independent
#3 (name/filename intern)  ──→  independent but uses same files as #2
#4 (PyFunction.name)  ──────→  independent
#5 (JIT default)  ──────────→  independent, one-line change
#6 (peephole)  ─────────────→  independent
#7 (zero-alloc iter)  ──────→  independent
#8 (intern LOAD_ATTR)  ─────→  independent, best effort last
```

Each item is independent. Recommended to do in numbered order for maximum impact/effort ratio, but no strict dependency.

---

## Expected Final State

| Metric | Before | After | Improvement |
|---|---|---|---|
| PyObject | 112 B | 96 B | -14% |
| CodeObject | 344 B | ~256 B | -26% |
| PyFunction | ~176 B | ~152 B | -14% |
| JIT speedup | N/A | ~5-50x on hot loops | Cranelift JIT |
| Dict/Set iteration | allocates per iter | zero-alloc | -100% allocs |
| LOAD_ATTR dispatch | string compare | int compare | faster |
| Compiler | emit only | emit + optimize | less runtime work |
