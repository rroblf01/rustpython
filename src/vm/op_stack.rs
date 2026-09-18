use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;

/// Extract the string data from a `str` object (covers the inline
/// `SmallStr` variant too — `borrow()` materializes it as `PyObject::Str`).
fn pyref_as_str(r: &PyObjectRef) -> Option<String> {
    match &*r.borrow() {
        PyObject::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

/// `cls.__match_args__`, or `None` if the class doesn't define it (plain
/// user classes without positionals). Mirrors the
/// `PyObject_GetOptionalAttr(type, "__match_args__")` step of CPython's
/// `_PyEval_MatchClass`.
fn lookup_match_args(cls: &PyObjectRef) -> Option<PyObjectRef> {
    cls.borrow().get_attribute("__match_args__").ok()
}

/// Which collection flavor a type carries, mirroring CPython's
/// mutually-exclusive `Py_TPFLAGS_SEQUENCE`/`Py_TPFLAGS_MAPPING` bits
/// (ground truth: `MATCH_SEQUENCE`/`MATCH_MAPPING` in
/// `Python/bytecodes.c` test exactly those bits — NOT isinstance, NOT
/// dunder presence: a plain class defining `__len__`+`__getitem__`
/// matches NEITHER gate on real CPython).
/// No type-flags field exists here, so the equivalent observable is
/// computed: the first MRO entry (in order) that is either a
/// natively-flagged builtin — list/tuple/range/array/memoryview (SEQ),
/// dict (MAP) — or defines `__abc_tpflags__` in its OWN dict (the
/// `Sequence`/`Mapping` ABCs via the vendored `_collections_abc.py`)
/// decides. This reproduces first-flagged-base inheritance
/// (`S1(UserList, Mapping)`→SEQ, `M1/M4(dict…, Sequence)`→MAP,
/// `X(C, Sequence)`→SEQ) and ABC registration is covered by the
/// isinstance fallback in the gates below (real CPython sets the bit
/// from `_abc._abc_register`; here the registry answers instead).
#[derive(PartialEq, Eq)]
enum CollectionFlavor {
    Sequence,
    Mapping,
}

fn collection_flavor(typ: &PyObjectRef) -> Option<CollectionFlavor> {
    let mro: Vec<PyObjectRef> = match &*typ.borrow() {
        PyObject::Type { mro, .. } => mro.clone(),
        _ => return None,
    };
    for entry in &mro {
        if let PyObject::Type { name, dict, .. } = &*entry.borrow() {
            // Natively-flagged builtins (name + home module, so a user
            // `class list` elsewhere can't spoof it; `array` lives in
            // module `array` on real CPython, `builtins` here — accept
            // both). Ground-truth table probed from `t.__flags__`.
            let module = entry
                .borrow()
                .get_attribute("__module__")
                .ok()
                .and_then(|m| pyref_as_str(&m));
            let is_builtin = module.as_deref() == Some("builtins");
            let is_collections = module.as_deref() == Some("collections");
            // Standalone reimplementations whose CPython originals inherit
            // the flag via ABCs (`UserList`/`deque` via `MutableSequence`;
            // `UserDict`/`ChainMap`/`OrderedDict` via `MutableMapping`) —
            // this interpreter keeps them deliberately ABC-free (see
            // collections_user_types.py), so the table confers exactly
            // what that inheritance would (probed from `t.__flags__`).
            let seq_native = is_builtin
                && matches!(name.as_str(), "list" | "tuple" | "range" | "memoryview")
                || matches!(module.as_deref(), Some("builtins" | "array"))
                    && name.as_str() == "array"
                || is_collections && matches!(name.as_str(), "deque" | "UserList");
            let map_native = is_builtin && name.as_str() == "dict"
                || is_collections
                    && matches!(name.as_str(), "UserDict" | "ChainMap" | "OrderedDict");
            if seq_native {
                return Some(CollectionFlavor::Sequence);
            }
            if map_native {
                return Some(CollectionFlavor::Mapping);
            }
            // ABC marker in the entry's OWN dict (32 = SEQ, 64 = MAP).
            if let Some(v) = dict.get_str("__abc_tpflags__") {
                if let PyObject::Int(n) = &*v.borrow() {
                    if *n == num_bigint::BigInt::from(32) {
                        return Some(CollectionFlavor::Sequence);
                    }
                    if *n == num_bigint::BigInt::from(64) {
                        return Some(CollectionFlavor::Mapping);
                    }
                }
            }
        }
    }
    None
}

/// `isinstance` against a `collections.abc` class — covers VIRTUAL
/// registration (`Sequence.register(Parent)`), which carries no MRO
/// marker. Misses (import failure, hook errors) mean "no match", never
/// an error: gates don't raise on real CPython either.
fn match_abc_isinstance(subj: &PyObjectRef, abc_name: &str) -> bool {
    let abc_mod = crate::modules::get_module("collections.abc").or_else(|| {
        crate::modules::get_module("collections").and_then(|m| m.borrow().get_attribute("abc").ok())
    });
    let abc = abc_mod.and_then(|m| m.borrow().get_attribute(abc_name).ok());
    match abc {
        Some(a) => crate::object::builtin_isinstance(&[subj.clone(), a])
            .map(|b| b.truthy())
            .unwrap_or(false),
        None => false,
    }
}

/// Whether `cls` carries CPython's `MATCH_SELF` semantics: a single
/// positional subpattern binds the subject itself. Ground truth:
/// `_Py_TPFLAGS_MATCH_SELF` on the builtin value types in
/// `Objects/{bytearray,bytes,dict,float,list,long,set,tuple,unicode}object.c`
/// (`bool` inherits it from `int`; `complex`/`range`/`object` lack it), and
/// subclasses keep it unless they redefine `__match_args__` (only called
/// when no MRO entry defines it, so that half is already settled).
/// A same-named user class (`class int: ...`, `__module__ != "builtins"`)
/// must NOT match — hence the module check per MRO entry.
fn type_has_match_self(cls: &PyObjectRef) -> bool {
    const SELF_TYPES: [&str; 10] = [
        "bytearray",
        "bytes",
        "dict",
        "float",
        "list",
        "int",
        "set",
        "frozenset",
        "tuple",
        "str",
    ];
    let mro: Vec<PyObjectRef> = match &*cls.borrow() {
        PyObject::Type { mro, .. } => mro.clone(),
        _ => return false,
    };
    for base in &mro {
        if let PyObject::Type { name, .. } = &*base.borrow() {
            if SELF_TYPES.contains(&name.as_str()) {
                let is_builtin = base
                    .borrow()
                    .get_attribute("__module__")
                    .ok()
                    .and_then(|m| pyref_as_str(&m))
                    .map(|m| m == "builtins")
                    .unwrap_or(false);
                if is_builtin {
                    return true;
                }
            }
        }
    }
    false
}

impl VirtualMachine {
    pub(crate) fn handle_stack(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<()> {
        match op {
            Opcode::POP_TOP => {
                self.frames[fi].pop()?;
            }

            Opcode::PRINT_EXPR => {
                // `compile(src, f, "single")` echo (REPL/doctest): print
                // the repr of any non-None expression-statement value to
                // the CURRENT sys.stdout (redirectable — never a raw
                // println! — matching `print`'s own lookup).
                let val = self.frames[fi].pop()?;
                if !matches!(&*val.borrow(), PyObject::None) {
                    let s = val.repr();
                    crate::object::print_with_vm(self, &[py_str(&s)], &[])?;
                }
            }

            Opcode::DUP_TOP => {
                let val = self.frames[fi].peek(0)?;
                self.frames[fi].push(val);
            }

            Opcode::COPY => {
                let depth = arg as usize;
                if depth >= self.frames[fi].stack.len() {
                    if let Some(val) = self.frames[fi].stack.last().cloned() {
                        self.frames[fi].push(val);
                    } else {
                        return Err(PyError::runtime_error("stack underflow (peek)"));
                    }
                } else {
                    let val = self.frames[fi].peek(depth)?;
                    self.frames[fi].push(val);
                }
            }

            Opcode::SWAP => {
                let i = arg as usize;
                let len = self.frames[fi].stack.len();
                if i > 0 && i < len {
                    self.frames[fi].stack.swap(len - 1, len - 1 - i);
                }
            }

            Opcode::GET_LEN => {
                let obj = self.frames[fi].pop()?;
                let len = crate::object::builtin_len(&[obj])?;
                self.frames[fi].push(len);
            }
            Opcode::MATCH_MAPPING => {
                // Flag-faithful gate (`type->tp_flags & Py_TPFLAGS_MAPPING`):
                // exact dicts, dict-flavored types (first MRO marker), or
                // virtual Mapping registrations. Plain instances never
                // match, even dict-likes without the flag.
                let subject = self.frames[fi].peek(0)?;
                let is_map = match &*subject.borrow() {
                    PyObject::Dict(_) => true,
                    PyObject::Instance { typ, .. } => match collection_flavor(typ) {
                        Some(CollectionFlavor::Mapping) => true,
                        Some(CollectionFlavor::Sequence) => false,
                        None => match_abc_isinstance(&subject, "Mapping"),
                    },
                    _ => false,
                };
                self.frames[fi].push(py_bool(is_map));
            }
            Opcode::MATCH_SEQUENCE => {
                // Flag-faithful gate (`type->tp_flags & Py_TPFLAGS_SEQUENCE`):
                // native sequences, sequence-flavored types, or virtual
                // Sequence registrations. str/bytes/bytearray carry
                // neither bit, so no explicit exclusion is needed; plain
                // dunder-only classes miss (real CPython agrees).
                let subject = self.frames[fi].peek(0)?;
                let is_seq = match &*subject.borrow() {
                    PyObject::List(_)
                    | PyObject::Tuple(_)
                    | PyObject::Range { .. }
                    | PyObject::Array(_)
                    | PyObject::MemoryView { .. } => true,
                    PyObject::Instance { typ, .. } => match collection_flavor(typ) {
                        Some(CollectionFlavor::Sequence) => true,
                        Some(CollectionFlavor::Mapping) => false,
                        None => match_abc_isinstance(&subject, "Sequence"),
                    },
                    _ => false,
                };
                self.frames[fi].push(py_bool(is_seq));
            }
            Opcode::MATCH_KEYS => {
                // Mapping key check: subject[key] must exist.
                // Stack: [.., subj, key] ; pops key, keeps subject.
                let key = self.frames[fi].pop()?;
                let subject = self.frames[fi].peek(0)?;
                let has_key = match &*subject.borrow() {
                    PyObject::Dict(d) => d.contains(&key)?,
                    _ => false,
                };
                self.frames[fi].push(py_bool(has_key));
            }
            Opcode::MATCH_CLASS_CHECK => {
                // Class-pattern funnel (CPython `_PyEval_MatchClass`
                // order): stack [.., subj, cls, kwd_tuple], arg =
                // positional count. Pops the kwd tuple; TypeErrors when
                // `cls` isn't a class; pushes isinstance(subj, cls);
                // validates `__match_args__` shape/count/collisions ONLY
                // on a match (a non-matching subject is a plain case
                // miss, never an error). Exits [.., subj, cls, bool].
                let npos = arg as usize;
                let kt = self.frames[fi].pop()?;
                let cls_h = self.frames[fi].peek(0)?;
                let subj_h = self.frames[fi].peek(1)?;
                let cls_name = match &*cls_h.borrow() {
                    PyObject::Type { name, .. } => name.clone(),
                    _ => {
                        return Err(PyError::type_error("called match pattern must be a class"));
                    }
                };
                // kwd attr names (built by our own codegen: always strings).
                let mut kwd_names: Vec<String> = Vec::new();
                if let PyObject::Tuple(items) = &*kt.borrow() {
                    for item in items {
                        if let Some(s) = pyref_as_str(item) {
                            kwd_names.push(s);
                        }
                    }
                }
                let matched = crate::object::builtin_isinstance(&[subj_h.clone(), cls_h.clone()])?;
                let matched = matched.truthy();
                self.frames[fi].push(py_bool(matched));
                if !matched {
                    return Ok(());
                }
                if npos > 0 {
                    match lookup_match_args(&cls_h) {
                        Some(match_args) => {
                            let items = match &*match_args.borrow() {
                                PyObject::Tuple(items) => items.clone(),
                                _ => {
                                    return Err(PyError::type_error(format!(
                                        "{}.__match_args__ must be a tuple (got {})",
                                        cls_name,
                                        match_args.borrow().type_name()
                                    )));
                                }
                            };
                            if items.len() < npos {
                                return Err(PyError::type_error(format!(
                                    "{}() accepts {} positional sub-pattern{} ({} given)",
                                    cls_name,
                                    items.len(),
                                    if items.len() == 1 { "" } else { "s" },
                                    npos
                                )));
                            }
                            let mut seen: std::collections::HashSet<String> =
                                std::collections::HashSet::new();
                            for item in items.iter().take(npos) {
                                let s = match pyref_as_str(item) {
                                    Some(s) => s,
                                    None => {
                                        return Err(PyError::type_error(format!(
                                            "__match_args__ elements must be strings (got {})",
                                            item.borrow().type_name()
                                        )));
                                    }
                                };
                                if !seen.insert(s.clone()) {
                                    return Err(PyError::type_error(format!(
                                        "{}() got multiple sub-patterns for attribute '{}'",
                                        cls_name, s
                                    )));
                                }
                            }
                            for k in &kwd_names {
                                if !seen.insert(k.clone()) {
                                    return Err(PyError::type_error(format!(
                                        "{}() got multiple sub-patterns for attribute '{}'",
                                        cls_name, k
                                    )));
                                }
                            }
                        }
                        None => {
                            // No `__match_args__` anywhere in the MRO:
                            // only MATCH_SELF builtins (`int`/`str`/`list`/
                            // ... and subclasses that don't redefine it)
                            // accept one positional, bound to the subject
                            // itself — anything else is "accepts 0".
                            if !type_has_match_self(&cls_h) {
                                return Err(PyError::type_error(format!(
                                    "{}() accepts 0 positional sub-patterns ({} given)",
                                    cls_name, npos
                                )));
                            }
                            if npos > 1 {
                                return Err(PyError::type_error(format!(
                                    "{}() accepts 1 positional sub-pattern ({} given)",
                                    cls_name, npos
                                )));
                            }
                            // npos == 1: self-match (kwargs, if any, can't
                            // collide — no positional names enter `seen`).
                        }
                    }
                }
            }
            Opcode::MATCH_CLASS_GETATTR => {
                // Positional extraction: stack [.., subj, cls], arg = index.
                // Peeks both; pushes getattr(subj, match_args[index])
                // (CPython binds positionals via __match_args__ names, not
                // by slot) — or the subject itself for MATCH_SELF builtins
                // (`case int(z)` binds `z = subject`). Exits
                // [.., subj, cls, attr].
                let idx = arg as usize;
                let cls_h = self.frames[fi].peek(0)?;
                let subj_h = self.frames[fi].peek(1)?;
                if lookup_match_args(&cls_h).is_none() {
                    // Self-match (CHECK already validated npos == 1 and
                    // the MATCH_SELF flag — only index 0 can arrive here).
                    if idx == 0 && type_has_match_self(&cls_h) {
                        self.frames[fi].push(subj_h);
                        return Ok(());
                    }
                    return Err(PyError::type_error("missing __match_args__".to_string()));
                }
                let match_args = lookup_match_args(&cls_h)
                    .ok_or_else(|| PyError::type_error("missing __match_args__".to_string()))?;
                let name: String = match &*match_args.borrow() {
                    PyObject::Tuple(items) => items.get(idx).and_then(pyref_as_str),
                    _ => None,
                }
                .ok_or_else(|| {
                    PyError::type_error("__match_args__ must be a tuple of strings".to_string())
                })?;
                // Precompute for the error message: holding `subj_h`'s
                // borrow across `get_attribute` (which can run arbitrary
                // Python) must not nest a second borrow inside `map_err`.
                let tname = subj_h.borrow().type_name();
                let attr = subj_h.borrow().get_attribute(&name).map_err(|_| {
                    PyError::attribute_error(format!(
                        "{} object has no attribute '{}'",
                        tname, name
                    ))
                })?;
                self.frames[fi].push(attr);
            }
            Opcode::CALL_INTRINSIC_1 => {
                let intrinsic = arg;
                match intrinsic {
                    1 => {
                        self.frames[fi].push(py_int(0));
                    }
                    2 => {
                        let val = self.frames[fi].pop()?;
                        let _ = print!("{}", val.str());
                        self.frames[fi].push(py_none());
                    }
                    _ => {
                        self.frames[fi].push(py_none());
                    }
                }
            }
            Opcode::CALL_INTRINSIC_2 => {
                self.frames[fi].push(py_int(0));
            }
            Opcode::UNPACK_SEQUENCE_TWO_TUPLE => {
                let seq = self.frames[fi].pop()?;
                let seq_borrowed = seq.borrow();
                if let PyObject::Tuple(items) = &*seq_borrowed {
                    if items.len() >= 2 {
                        self.frames[fi].push(items[0].clone());
                        self.frames[fi].push(items[1].clone());
                    } else {
                        return Err(PyError::runtime_error("not enough values to unpack"));
                    }
                } else if let PyObject::List(items) = &*seq_borrowed {
                    if items.len() >= 2 {
                        self.frames[fi].push(items[0].clone());
                        self.frames[fi].push(items[1].clone());
                    } else {
                        return Err(PyError::runtime_error("not enough values to unpack"));
                    }
                } else {
                    let it = crate::object::builtin_iter(&[seq.clone()])?;
                    let v1 = crate::object::builtin_next(&[it.clone()])?;
                    let v2 = crate::object::builtin_next(&[it.clone()])?;
                    self.frames[fi].push(v1);
                    self.frames[fi].push(v2);
                }
            }

            _ => {}
        }
        Ok(())
    }
}
