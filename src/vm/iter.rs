use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    pub(crate) fn handle_iter(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::GET_ITER => {
                let val = self.frames[fi].pop()?;
                // Check for user-class instance (needs __iter__ protocol)
                let is_instance = val.borrow().type_name() == "instance";
                if is_instance {
                    // A class transparently subclassing list/dict/str
                    // (`class Foo(list): ...`) with no __iter__ override
                    // should iterate its real native backing directly —
                    // list/dict don't define "__iter__" as a plain
                    // get_attribute entry (iteration normally goes through
                    // this same opcode's native match instead), so routing
                    // it through get_attribute below would silently miss and
                    // fall into the unrelated dict-like-instance fallback.
                    let has_override = if let PyObject::Instance { typ, .. } = &*val.borrow() {
                        crate::object::lookup_dunder_via_mro(typ, "__iter__").is_some()
                    } else {
                        false
                    };
                    if !has_override {
                        if let Some(native) = crate::object::native_backing_of(&val) {
                            let iterator = crate::object::builtin_iter(&[native])?;
                            self.frames[fi].push(iterator);
                            return Ok(true);
                        }
                        // No `__iter__` override (confirmed via the mro
                        // lookup above) and no native backing — delegate
                        // to `builtin_iter`, which implements the real
                        // "no `__iter__`, fall back to `__getitem__`"
                        // protocol (`for x in obj:` calling `obj[0]`,
                        // `obj[1]`, ... until `IndexError`) and raises a
                        // clean `TypeError` if neither exists. Previously
                        // this fell through to `get_attribute("__iter__")`
                        // below even with `has_override` already known
                        // false, which doesn't raise cleanly for a plain
                        // instance with no `__iter__` — real trigger:
                        // `for x in SequenceClass(3): ...` (an object with
                        // only `__getitem__`, the standard old-style
                        // sequence-iteration idiom) silently misbehaved
                        // instead of iterating 0, 1, 2.
                        let iterator = crate::object::builtin_iter(&[val.clone()])?;
                        self.frames[fi].push(iterator);
                        return Ok(true);
                    }
                    use crate::object::ObjectAccess;
                    let raw_method = val.borrow().get_attribute("__iter__").map_err(|_| {
                        PyError::type_error(format!(
                            "'{}' object is not iterable",
                            val.borrow().type_name()
                        ))
                    })?;
                    let val_clone = val.clone();
                    let iter_method = PyObjectRef::imm(PyObject::BoundMethod {
                        func: raw_method,
                        self_obj: val_clone,
                    });
                    // LAZY iteration (CPython semantics): `__iter__` returns
                    // THE iterator; FOR_ITER advances it one step at a time.
                    let iterator = self.call_function(iter_method, vec![], vec![])?;
                    self.frames[fi].push(iterator);
                } else {
                    let obj = val.borrow();
                    match &*obj {
                        PyObject::List(v) => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: v.clone(),
                                index: 0,
                            }));
                        }
                        PyObject::Deque { data, .. } => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::DequeIter {
                                deque: val.clone(),
                                index: 0,
                                start_len: data.len(),
                            }));
                        }
                        PyObject::Tuple(v) => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: v.clone(),
                                index: 0,
                            }));
                        }
                        PyObject::Str(s) => {
                            let chars: Vec<PyObjectRef> =
                                s.chars().map(|c| py_str(&c.to_string())).collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: chars,
                                index: 0,
                            }));
                        }
                        // `FrozenSet` was missing from this match entirely
                        // (only mutable `Set` was handled) — `for x in
                        // frozenset(...):`/`for x in some_frozenset:` fell to
                        // the `_` catch-all below and raised `TypeError:
                        // 'frozenset' object is not iterable` outright, a
                        // foundational gap for one of Python's basic builtin
                        // container types. `builtin_iter`'s OWN, separate
                        // FrozenSet handling (used by `iter()`/`list()`/etc.,
                        // not by a `for` STATEMENT, which compiles to this
                        // opcode instead) had the identical gap, fixed
                        // alongside this one.
                        PyObject::Set(s) | PyObject::FrozenSet(s) => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: s.to_vec(),
                                index: 0,
                            }));
                        }
                        PyObject::Bytes(b) => {
                            let items: Vec<PyObjectRef> =
                                b.iter().map(|byte| py_int(*byte as i64)).collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: items,
                                index: 0,
                            }));
                        }
                        PyObject::ByteArray(b) => {
                            let items: Vec<PyObjectRef> =
                                b.iter().map(|byte| py_int(*byte as i64)).collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: items,
                                index: 0,
                            }));
                        }
                        PyObject::Array(arr) => {
                            let items: Vec<PyObjectRef> = arr
                                .data
                                .iter()
                                .map(|v| {
                                    if crate::object::array_typecode_is_float(arr.typecode) {
                                        py_float(*v)
                                    } else if arr.typecode == 'w' || arr.typecode == 'u' {
                                        let ch = (*v as u32).try_into().ok().and_then(char::from_u32).unwrap_or('\0');
                                        py_str(&ch.to_string())
                                    } else {
                                        py_int(*v as i64)
                                    }
                                })
                                .collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: items,
                                index: 0,
                            }));
                        }
                        PyObject::MemoryView { .. } => {
                            drop(obj);
                            let iterator = crate::object::builtin_iter(&[val.clone()])?;
                            self.frames[fi].push(iterator);
                        }
                        PyObject::Generator { .. } => {
                            drop(obj);
                            self.frames[fi].push(val);
                        }
                        PyObject::Range { start, stop, step } => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::RangeIter {
                                current: start.clone(),
                                stop: stop.clone(),
                                step: step.clone(),
                            }));
                        }
                        PyObject::Dict(ref pydict) => {
                            let keys: Vec<PyObjectRef> = pydict.keys();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: keys,
                                index: 0,
                            }));
                        }
                        PyObject::Globals(g) => {
                            let keys: Vec<PyObjectRef> = g
                                .borrow()
                                .keys()
                                .map(|k| py_str(crate::interner::lookup_str(*k)))
                                .collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: keys,
                                index: 0,
                            }));
                        }
                        PyObject::EnumerateIter { .. } => {
                            drop(obj);
                            self.frames[fi].push(val);
                        }
                        // Iterators are their own iterator (matching CPython's
                        // `__iter__` returning self) — `for x in iter(y):` or
                        // `for x in itertools.tee(y)[0]:` must work the same as
                        // iterating the original iterable directly.
                        PyObject::ListIter { .. }
                        | PyObject::RangeIter { .. }
                        | PyObject::MapIterator { .. }
                        | PyObject::FilterIterator { .. }
                        | PyObject::ZipIterator { .. }
                        | PyObject::CycleIter { .. }
                        | PyObject::GroupByIter { .. }
                        | PyObject::GetItemIter { .. }
                        | PyObject::CallSentinelIter { .. } => {
                            drop(obj);
                            self.frames[fi].push(val);
                        }
                        // A class object itself can be iterable via its
                        // metaclass's `__iter__` (e.g. `for member in
                        // SomeEnum:` — `SomeEnum` is a `PyObject::Type`, and
                        // `__iter__` lives on its metaclass, not on `SomeEnum`
                        // own dict/mro, which is why this needs metatype_of
                        // rather than the ordinary Type attribute lookup above).
                        PyObject::Type { .. } => {
                            let iter_fn = crate::object::metatype_of(&val).and_then(|mt| {
                                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                                    for base in mro.iter() {
                                        if let PyObject::Type { dict, .. } = &*base.borrow() {
                                            if let Some(v) = dict.get_str("__iter__") {
                                                return Some(v.clone());
                                            }
                                        }
                                    }
                                }
                                None
                            });
                            drop(obj);
                            match iter_fn {
                                Some(f) => {
                                    let iterator =
                                        self.call_function(f, vec![val.clone()], vec![])?;
                                    self.frames[fi].push(iterator);
                                }
                                None => {
                                    return Err(PyError::type_error(format!(
                                        "'{}' object is not iterable",
                                        val.get_type_name()
                                    )))
                                }
                            }
                        }
                        // `for line in open(path):` — one of the single most
                        // common real-Python file-reading idioms — was entirely
                        // unhandled (`TypeError: 'file' object is not
                        // iterable`), confirmed via `Lib/dbm/dumb.py`'s own
                        // `_update` (`for line in f:` over its index file), but
                        // the gap is completely general, not dbm-specific.
                        // Reads the whole remaining content and splits it into
                        // lines (keeping each line's own trailing `\n`, matching
                        // real `readline()`/iteration semantics) — eager,
                        // matching every other native-type arm in this same
                        // match (`List`/`Tuple`/`Str`/...), not the lazy
                        // `CallSentinelIter` `readline()`-driven approach used
                        // by this project's OWN `readline`/`__next__` methods
                        // (added alongside this fix, `attrs.rs`) for direct
                        // `f.readline()`/`next(f)` calls.
                        PyObject::File { file, binary, .. } => {
                            use std::io::Read;
                            let binary = *binary;
                            let mut rest = Vec::new();
                            file.borrow_mut()
                                .read_to_end(&mut rest)
                                .map_err(|e| PyError::os_error_from_io(&e))?;
                            drop(obj);
                            let mut lines: Vec<PyObjectRef> = Vec::new();
                            let mut current: Vec<u8> = Vec::new();
                            for byte in rest {
                                current.push(byte);
                                if byte == b'\n' {
                                    lines.push(if binary {
                                        PyObjectRef::imm(PyObject::Bytes(current.clone()))
                                    } else {
                                        py_str(&String::from_utf8_lossy(&current))
                                    });
                                    current.clear();
                                }
                            }
                            if !current.is_empty() {
                                lines.push(if binary {
                                    PyObjectRef::imm(PyObject::Bytes(current.clone()))
                                } else {
                                    py_str(&String::from_utf8_lossy(&current))
                                });
                            }
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: lines,
                                index: 0,
                            }));
                        }
                        _ => {
                            let type_name = obj.type_name();
                            drop(obj);
                            match crate::object::builtin_iter(&[val.clone()]) {
                                Ok(it) => self.frames[fi].push(it),
                                Err(_) => {
                                    return Err(PyError::type_error(format!(
                                        "'{}' object is not iterable",
                                        type_name
                                    )))
                                }
                            }
                        }
                    }
                }
            }

            Opcode::FOR_ITER => {
                let iter_val = self.frames[fi].peek(0)?;
                let is_generator = matches!(&*iter_val.borrow(), PyObject::Generator { .. });
                if is_generator {
                    // Call __next__ on generator
                    let gen = iter_val.clone();
                    let next_func = gen.borrow().get_attribute("__next__");
                    if let Ok(next_func) = next_func {
                        // Fix self_obj by extracting name and func
                        let (n, f) = {
                            let b = next_func.borrow();
                            if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                (name.clone(), *func)
                            } else {
                                return Err(PyError::runtime_error("expected __next__ method"));
                            }
                        };
                        let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: n,
                            func: f,
                            self_obj: gen.clone(),
                        });
                        match self.call_function(fixed, vec![], vec![]) {
                            Ok(val) => {
                                self.frames[fi].push(val);
                            }
                            // A generator's __next__/send driver signals
                            // normal exhaustion via an ad hoc
                            // `PyError::Exception("StopIteration", return_value)`
                            // (see its get_attribute arm), not the plain
                            // `PyError::StopIteration` variant — checking
                            // only the latter here meant `for x in
                            // some_generator(): ...` never terminated
                            // cleanly and instead leaked as an uncaught
                            // exception once the generator was exhausted.
                            Err(e) if crate::object::is_stop_iteration_error(&e) => {
                                self.frames[fi].ip = arg as usize;
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        self.frames[fi].ip = arg as usize;
                    }
                } else {
                    let is_exhausted = {
                        let obj = iter_val.borrow();
                        match &*obj {
                            PyObject::List(v) => v.is_empty(),
                            PyObject::ListIter { list, index } => *index >= list.len(),
                            PyObject::RangeIter {
                                current,
                                stop,
                                step,
                            } => {
                                if step.sign() == num_bigint::Sign::Plus {
                                    *current >= *stop
                                } else {
                                    *current <= *stop
                                }
                            }
                            // ZipIterator/MapIterator/FilterIterator don't fit
                            // this branch's exhausted-check-then-advance shape
                            // (advancing several sub-iterators, e.g. zip's, in
                            // lockstep isn't a simple index/length compare) —
                            // `builtin_next` already implements all of that
                            // correctly (it's what list()/sum()/etc. already go
                            // through), so drop straight into it here instead
                            // of duplicating that logic. Previously these three
                            // fell to the `_` arm below and raised "for_iter on
                            // non-iterable" — i.e. `for x in zip(a, b):` (or
                            // map/filter) used directly as a for-loop target,
                            // as opposed to being wrapped in `list(...)` first,
                            // has never worked.
                            // `CycleIter` (`itertools.cycle`) shares the same
                            // "doesn't fit index/length exhaustion" shape —
                            // genuinely infinite (wraps via modulo), so there's
                            // no `len()` to compare against at all; delegate to
                            // `builtin_next` exactly like Zip/Map/Filter above.
                            // `EnumerateIter` moved here too — it no longer
                            // holds a materialized `items`/`len()` to compare
                            // against now that it's a lazy wrapper around a
                            // `source` iterator (see its own doc comment).
                            PyObject::ZipIterator { .. }
                            | PyObject::MapIterator { .. }
                            | PyObject::FilterIterator { .. }
                            | PyObject::CycleIter { .. }
                            | PyObject::EnumerateIter { .. }
                            | PyObject::GroupByIter { .. }
                            | PyObject::GetItemIter { .. }
                            | PyObject::CallSentinelIter { .. }
                            | PyObject::DequeIter { .. }
                            | PyObject::DequeRevIter { .. } => {
                                drop(obj);
                                match crate::object::builtin_next(&[iter_val.clone()]) {
                                    Ok(val) => {
                                        self.frames[fi].push(val);
                                    }
                                    Err(e) if crate::object::is_stop_iteration_error(&e) => {
                                        self.frames[fi].ip = arg as usize;
                                    }
                                    Err(e) => return Err(e),
                                }
                                return Ok(true);
                            }
                            _ => {
                                // Not a built-in iterator — check for __next__ protocol.
                                // MUST drop(obj) before delegating: `obj` is a live
                                // borrow of the iterator, and __next__ legitimately
                                // mutates it (self.i += 1 in user code) -> borrow_mut
                                // on the same object while this borrow is held
                                // panicked with "RefCell already borrowed" for ANY
                                // `yield from custom_iter` / `for x in custom:`
                                // whose __next__ stores attributes.
                                let is_inst = obj.type_name() == "instance";
                                drop(obj);
                                if is_inst {
                                    self.for_iter_next(iter_val.clone(), arg)?;
                                    return Ok(true);
                                }
                                return Err(PyError::type_error("for_iter on non-iterable"));
                            }
                        }
                    };
                    if is_exhausted {
                        self.frames[fi].ip = arg as usize;
                    } else {
                        let val = self.frames[fi].pop()?;
                        let item = {
                            // Convert plain List to ListIter for O(1) iteration
                            let is_plain_list = matches!(&*val.borrow(), PyObject::List(..));
                            if is_plain_list {
                                let list_clone = {
                                    let obj = val.borrow();
                                    if let PyObject::List(v) = &*obj {
                                        v.clone()
                                    } else {
                                        unreachable!()
                                    }
                                };
                                *val.borrow_mut() = PyObject::ListIter {
                                    list: list_clone,
                                    index: 0,
                                };
                            }
                            let mut obj = val.borrow_mut();
                            match &mut *obj {
                                PyObject::ListIter { list, index } => {
                                    let v = list[*index].clone();
                                    *index += 1;
                                    v
                                }
                                PyObject::RangeIter {
                                    current,
                                    stop: _,
                                    step,
                                } => {
                                    let v = py_int(current.clone());
                                    *current += &*step;
                                    v
                                }
                                // `EnumerateIter` no longer reaches this arm at
                                // all — it moved to the earlier "delegate to
                                // builtin_next, return early" bucket above
                                // (alongside Zip/Map/Filter/Cycle) once it
                                // became a lazy `source`-wrapper instead of a
                                // materialized `items` list with no `len()` to
                                // compare against.
                                _ => unreachable!(),
                            }
                        };
                        self.frames[fi].push(val);
                        self.frames[fi].push(item);
                    }
                }
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
