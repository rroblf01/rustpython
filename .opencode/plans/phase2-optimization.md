# Phase 2: Closing the Gap with CPython

## Diagnosis

| Metric | CPython | RustPython | Gap |
|--------|---------|------------|-----|
| RAM (delta) | 87 MB | 105 MB | +20% |
| Speed (total) | 0.18s | 1.80s | ~10× |

## Root Causes

### RAM (> CPython by ~18 MB)

1. **HashMap<String, PyObjectRef>** for globals, builtins, type dicts:
   Each String key = 24B + heap allocation (~20-60B per key). CPython uses interned
   PyObject* pointers (8B). With thousands of entries, this adds up to ~4-8 MB.

2. **AttrMap with Vec<(String, PyObjectRef)>**:
   Same String overhead for instance attribute keys. ~2-4 MB.

3. **Frame pool caches retain capacity**: attr_cache/global_cache Vecs are
   `.clear()`ed on frame reuse but keep their allocated capacity. ~0.5-2 MB.

### Speed (10× behind CPython)

**Primary bottleneck (estimated 5-7×): Inline caches disabled**
- `attr_cache` and `global_cache` are permanently disabled on frame reuse
  (`.clear()` → len=0 → always miss)
- Every LOAD_ATTR redoes the full ~500-line dispatch
- Every LOAD_GLOBAL does 3 HashMap lookups from scratch
- CPython hits ~90% of LOAD_ATTR via inline cache in ~3 instructions

**Secondary bottleneck (estimated 2-3×): Object model overhead**
- `RefCell::borrow()` on every object access (runtime borrow tracking)
- String comparisons in attribute dispatch (vs StrId comparisons)
- Flat `match op` dispatch (vs computed-goto in CPython)

---

## Optimization Plan (8 items, priority order)

### S1: Fix inline caches with generation counters ⭐⭐⭐
**Speed: estimated 3-5× improvement. RAM: neutral.**

The `attr_cache` stores `Option<(u64, PyObjectRef)>` where the u64 is already
intended as a type version tag. Currently it uses `typ.get_id() as u64` (the
object's pointer, which doesn't change on mutation). Fix:

1. Add `version: Cell<u64>` to `PyObject::Type`
2. Increment on every `set_attribute`/`del_attribute` on a type
3. Use the type's version as the cache tag instead of the pointer
4. In `acquire_frame`, resize caches to `code.instructions.len()` instead of
   `.clear()` (enables cache hits on pooled frames)

### R1: StrId for HashMap keys ⭐⭐⭐
**RAM: estimated 4-8 MB savings. Speed: secondary (faster HashMap lookups).**

Change `HashMap<String, PyObjectRef>` → `HashMap<StrId, PyObjectRef>` in:
- Frame globals
- Frame builtins
- Type dicts (in PyObject::Type)
- Module dicts (in PyObject::Module)
- PyFunction globals

### R2: StrId for AttrMap keys ⭐⭐
**RAM: estimated 2-4 MB savings.**

Change `Vec<(String, PyObjectRef)>` → `Vec<(StrId, PyObjectRef)>` in AttrMap.

### S2: Pre-compute StrId for attribute comparisons ⭐⭐
**Speed: estimated 1.2-1.5× improvement for LOAD_ATTR.**

In LOAD_ATTR dispatch, pre-compute StrId for all string literals compared
against (__dict__, __class__, __get__, __set__, fget, fset, etc.).

### R3: Fix frame pool cache resize ⭐
**RAM: estimated 0.5-2 MB savings. Speed: enables S1.**

Currently `attr_cache.clear()` and `global_cache.clear()` just set len=0 but
keep capacity. Change to resize to the new code's instr count.

### S3: Bypass RefCell for immutable objects ⭐
**Speed: estimated 1.2-1.5×.**

Use `Imm` variant's guarantee of immutability to skip `borrow()` overhead
for types like Int, Str, Tuple, Code, etc.

### S4: Optimized small function dispatch ⭐
**Speed: estimated 1.1-1.3×.**

The `try_exec_simple` path already avoids frame allocation for tiny functions.
Extend it with tail-call-style dispatch for tiny recursive calls.

### R4: Compact type representation ⭐
**RAM: estimated 1-2 MB.**

Currently `Type` stores `{name, dict, bases, mro}` inline (80B in PyObject).
Box the full Type struct to 8B pointer.
