# RustPython — Architecture & Roadmap v2

## Status: Phases 1-5 Implemented

| Phase | Component | Status | Benchmark Impact |
|-------|-----------|--------|------------------|
| 1 | String interning + InternedMap | ✅ `src/interner.rs` | Reduces name lookup allocs |
| 2 | JIT extended (3→11 ops) | ✅ `src/jit.rs` | Bitwise ops native, div/mod/pow via FFI |
| 3 | Inline cache (LOAD_GLOBAL) | ✅ `src/vm.rs` | Cache global lookups per instruction offset |
| 4 | SmallVec stack | ✅ `src/vm.rs` | ~7% faster (0.136→0.127s) |
| 5 | Tagged pointers (SmallFloat, SmallStr) | ✅ `src/object.rs` | Avoids Rc+heap for floats + short strings |
| **Current total** | | | **~3.2× slower than CPython** (was 6-18×) |

---

## Phase 6: Generational GC (`src/gc.rs`) — Estimated impact: 2-5× faster

### Current problem
`Rc<RefCell<PyObject>>` for every mutable object has three costs:
1. **RefCell runtime checks** — every borrow()/borrow_mut() checks the borrow flag
2. **No cycle detection** — cycles between objects (common in Python) never free
3. **High allocation overhead** — every PyObject is a separate heap allocation

### Architecture

```
┌─────────────────────────┐
│  GcHeap                 │
│  ├── young_gen (bump)   │  ← eden: bump-allocate, sweep frequently
│  ├── old_gen (mark-sweep)│  ← promoted after surviving 2 young GCs
│  └── large_object_space  │  ← huge objects (>64KB)
└─────────────────────────┘
```

### Key design

```rust
pub struct GcHeap {
    young: BumpAllocator,    // Thread-local bump allocation
    old: MarkSweepAllocator, // Mark-sweep with tri-color marking
    roots: Vec<GcRoot>,      // Stack roots + globals
}

pub struct GcObj<T> {
    header: GcHeader,         // Mark bits, type tag, size
    data: UnsafeCell<T>,      // No RefCell needed — GC manages mutation
}

// Replace PyObjectRef::Mut(Rc<RefCell<PyObject>>)
// with:
//   PyObjectRef::Gc(GcObj<PyObject>)
```

### Benefits
- **Bump allocation** for young objects (O(1), no malloc)
- **Cycle detection** via tri-color marking
- **No RefCell** borrow-check overhead
- **Parallel marking** with rayon work-stealing

### Implementation steps
1. Implement `GcHeader` with mark bits + type tag
2. Implement `BumpAllocator` for young generation
3. Implement `MarkSweepAllocator` with tri-color marking
4. Add `GcRoot` stack-scanning interface
5. Replace `Rc<RefCell<PyObject>>` with `GcObj<PyObject>` in PyObjectRef
6. Add GC root scanning in VM frames

---

## Phase 7: Register-Based Bytecode (`src/compiler.rs` + `src/vm.rs`) — Estimated impact: 2-4× faster

### Current problem
Stack-based bytecode requires push/pop for every operation. Register-based
encoding reduces instruction count by ~30% and enables better JIT codegen.

### Design

```rust
// Current: STACK-BASED
//   LOAD_CONST 0    # push 1
//   LOAD_CONST 1    # push 2  
//   BINARY_OP 0     # pop 2 → pop 1 → add → push

// New: REGISTER-BASED (local, 256 virtual registers)
//   MOVI r0, 1
//   MOVI r1, 2
//   ADD  r2, r0, r1   # r2 = r0 + r1
```

### Dual-mode VM architecture

```rust
enum BytecodeMode {
    Stack,     // Current — for compatibility
    Register,  // New — for hot paths
}

enum Instruction {
    // Stack-mode (existing)
    LOAD_CONST { arg: u32 },
    BINARY_OP { op: u8 },
    // Register-mode (new)
    MOV_REG { dst: u8, src: u8 },
    ADD { dst: u8, a: u8, b: u8 },
    CALL { dst: u8, func: u8, nargs: u8 },
}
```

### Optimizations
- **Register allocation**: simple linear scan over bytecode
- **SSA form**: each register assigned exactly once for JIT
- **Dead code elimination**: remove unused register writes

---

## Phase 8: C API Bridge (`src/ffi_bridge.rs`) — Enables CPython extension compatibility

### Architecture

```
┌──────────────────────┐
│  C Extension (.so)   │  (psycopg2, numpy, lxml, etc.)
├──────────────────────┤
│  CPython C API shim  │  ~200 functions
├──────────────────────┤
│  RustPython objects  │
└──────────────────────┘
```

### Key components

```rust
// Memory layout compatibility
#[repr(C)]
struct PyObject_Header {
    ob_refcnt: usize,
    ob_type: *mut PyTypeObject,
}

// C API functions to implement
impl CAPIBridge {
    fn PyArg_ParseTuple(args: *mut PyObject, format: *const c_char, ...) -> c_int;
    fn PyObject_GetAttr(obj: *mut PyObject, name: *mut PyObject) -> *mut PyObject;
    fn PyObject_SetAttr(obj: *mut PyObject, name: *mut PyObject, val: *mut PyObject) -> c_int;
    fn PyBool_Check(obj: *mut PyObject) -> c_int;
    fn PyLong_AsLong(obj: *mut PyObject) -> c_long;
    // ... ~200 more
}
```

### Strategy
1. Use `libloading` crate to dynamically load `.so` files
2. Expose `PyInit_<module>` symbol resolution
3. Wrap RustPython objects with `#[repr(C)]` layout matching CPython's `PyObject`
4. Maintain a mapping of active module handles
5. Thread-safety via per-module mutex

---

## Phase 9: Thread-Safe Objects (`Arc<Mutex<>>`) — Enables real parallelism

### Current problem
`Rc` is not `Send`/`Sync`. `RefCell` is not `Sync`. This means RustPython
cannot run multiple threads sharing objects — a hard requirement for WSGI/ASGI
servers and Django.

### Migration path

```rust
// Phase 9a — Per-thread interpreter (easiest)
// Each thread gets its own VM with no shared state
// Already works with current Rc<RefCell>

// Phase 9b — Shared immutable objects
// Replace Imm(Rc<PyObject>) with Imm(Arc<PyObject>)
// Arc is Send+Sync, same performance as Rc for reads

// Phase 9c — Shared mutable objects
// Replace Mut(Rc<RefCell<PyObject>>) with Mut(Arc<RwLock<PyObject>>)
// RwLock allows concurrent reads, exclusive writes
```

### Performance considerations
- `Arc` adds ∼8 bytes overhead vs `Rc` (negligible)
- `RwLock` is uncontended in single-threaded (∼25ns acquire)
- Use CAS-based reference counting instead of atomic increment
- GIL-free parallelism via work-stealing thread pool

---

## Phase 10: Profile-Guided JIT (`src/jit.rs` v2)

### Design

```rust
struct HotspotDetector {
    counters: Vec<u32>,  // Per-bytecode-offset execution count
    threshold: u32,      // Offload to JIT after N executions
}

impl JitCompiler {
    fn compile_hot(code: &CodeObject, 
                   profile: &[u32]) -> Option<JitFn> {
        // Use profile data to:
        // 1. Inline commonly called functions
        // 2. Unroll hot loops
        // 3. Specialize types (PIC for attribute lookups)
        // 4. Allocate registers for most-used locals
    }
}
```

### Type specialization
```rust
// Profile common value types at each instruction
struct TypeProfile {
    small_int_pct: f32,   // % of times value was SmallInt
    small_float_pct: f32, // % of times value was SmallFloat
    str_pct: f32,         // % of times value was Str
}

// Generate specialized JIT code for the most common type
// Fall back to generic path when type prediction fails
```

### Hot loop inlining
- Track call/return edges in profile
- Inline functions called >1000 times in hot loops
- Eliminate frame allocation for inlined calls

### Expected impact
Combined with Phases 6-7: **2-8× faster than CPython** for numerical code,
**1.5-3× faster** for general Python.

---

## Summary

| Phase | Effort | Speed Impact | Memory Impact |
|-------|--------|-------------|---------------|
| 1-5 ✅ | ~2 days | ~2-3× faster | -20% allocs |
| 6 (GC) | ~2 weeks | 2-5× faster | -50% memory, no leaks |
| 7 (reg VM) | ~1 week | 2-4× faster | -10% instr count |
| 8 (C API) | ~4 weeks | N/A (compat) | Depends on exts |
| 9 (threads) | ~2 weeks | 4-8× on multi-core | +5% per-object |
| 10 (PGO JIT) | ~3 weeks | 2-4× faster | +1% profile data |
| **Total** | **~12 weeks** | **Target: 2-8× faster than CPython** | |
