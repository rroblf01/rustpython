use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use smallvec::SmallVec;
use crate::bytecode::*;
use crate::interner::{self, StrId, InternedMap};

use crate::modules::*;
use crate::object::*;
use crate::parser::Parser;
use crate::compiler::Compiler;
#[cfg(feature = "jit")]
use crate::jit::JitCompiler;

thread_local! {
    static ATTR_CACHE: std::cell::RefCell<HashMap<(String, String), crate::object::BuiltinFunc>> = std::cell::RefCell::new(HashMap::new());
}

#[derive(Clone)]
pub struct Frame {
    pub code: Rc<CodeObject>,
    pub locals: InternedMap<PyObjectRef>,
    pub fast_locals: Vec<Option<PyObjectRef>>,
    pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
    pub builtins: Rc<HashMap<StrId, PyObjectRef>>,
    pub stack: SmallVec<[PyObjectRef; 4]>,
    pub ip: usize,
    pub base_sp: usize,
    pub exception_handlers: Box<Vec<ExceptionHandler>>,
    pub closure: Box<Vec<PyObjectRef>>,
    /// Active exception for re-raise. Set by PUSH_EXC_INFO, consumed by RERAISE.
    /// This is separate from the value stack so that POP_EXCEPT (which pops the
    /// exception from the value stack) does not break RERAISE in try/finally blocks.
    pub active_exception: Option<Box<PyObjectRef>>,
    /// Inline attribute cache — caches LOAD_ATTR results per instruction offset.
    /// Cleared when the frame is created; populated on first attribute access.
    pub attr_cache: Box<Vec<Option<(u64, PyObjectRef)>>>,  // (type_version_tag, cached_value)
    /// Inline global cache — caches LOAD_GLOBAL results per instruction offset.
    pub global_cache: Box<Vec<Option<PyObjectRef>>>,
    /// Virtual registers for register-based bytecode execution.
    /// 256 virtual registers (u8 index) — no stack needed for most ops.
    pub registers: Box<Vec<Option<PyObjectRef>>>,
    /// Optional reference to the enclosing module's globals.
    /// Used by class bodies to resolve LOAD_NAME against module-level names
    /// and by MAKE_FUNCTION to set __module__ on created functions.
    pub module_globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
    /// First-insertion order of names STORE_NAME'd into this frame's
    /// `globals` — only populated for class-body frames (set up by
    /// `__build_class__`), since that's the one case where order is
    /// user-visible (class namespaces, and anything a metaclass inspects,
    /// e.g. enum member definition order). `globals` itself is a plain
    /// HashMap with no ordering guarantee; `None` for ordinary module/
    /// function frames, where nothing currently depends on order and
    /// tracking it would be pure overhead.
    pub name_order: Option<Rc<RefCell<Vec<String>>>>,
    /// The PyObject::Module this frame is the top-level execution of, if any.
    /// A module's own `dict` is otherwise only synced from `globals` once
    /// the whole body finishes executing (see `exec_module_source`) — so
    /// any attribute access on the module object *while it's still
    /// mid-execution* (e.g. a circular import reading a name defined
    /// earlier in the same file) would see a stale/empty dict. Real
    /// CPython avoids this because `module.__dict__` IS the executing
    /// frame's globals, not a separate copy. Mirroring every STORE_NAME
    /// into this live module's dict (see STORE_NAME/DELETE_NAME) gives the
    /// same effect generally, for every module, not just via IMPORT_FROM's
    /// narrower ancestor-frame fallback.
    pub live_module: Option<PyObjectRef>,
}

#[derive(Clone)]
pub struct ExceptionHandler {
    pub instr_addr: usize,
    pub stack_depth: usize,
}

impl Frame {
    pub fn new(
        code: Rc<CodeObject>,
        globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
        builtins: Rc<HashMap<StrId, PyObjectRef>>,
        module_globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
    ) -> Self {
        let instr_count = code.instructions.len();
        let names_len = code.names.len();
        Frame {
            fast_locals: vec![None; code.nlocals],
            code,
            locals: InternedMap::new(),
            globals,
            builtins,
            stack: SmallVec::new(),
            ip: 0,
            base_sp: 0,
            exception_handlers: Box::new(Vec::new()),
            closure: Box::new(Vec::new()),
            active_exception: None,
            attr_cache: Box::new(vec![None; names_len]),
            global_cache: Box::new(vec![None; instr_count]),
            registers: Box::new(Vec::new()),
            module_globals,
            name_order: None,
            live_module: None,
        }
    }

    pub fn push(&mut self, obj: PyObjectRef) {
        self.stack.push(obj);
    }

    pub fn pop(&mut self) -> PyResult<PyObjectRef> {
        self.stack.pop().ok_or_else(|| {
            let instr_ip = if self.ip > 0 { self.ip - 1 } else { 0 };
            let op_str = if instr_ip < self.code.instructions.len() {
                format!("{:?}", self.code.instructions[instr_ip].op)
            } else {
                "END".to_string()
            };
            let arg = if instr_ip < self.code.instructions.len() { self.code.instructions[instr_ip].arg } else { 0 };
            let line_no = if instr_ip < self.code.instructions.len() { self.code.line_number(instr_ip) } else { 0 };
            PyError::runtime_error(format!("stack underflow at {} arg={} line={} code={} file={}", op_str, arg, line_no, self.code.name, self.code.filename))
        })
    }

    pub fn peek(&self, depth: usize) -> PyResult<PyObjectRef> {
        if depth >= self.stack.len() {
            let instr_ip = if self.ip > 0 { self.ip - 1 } else { 0 };
            let _op_str = if instr_ip < self.code.instructions.len() {
                format!("{:?}", self.code.instructions[instr_ip].op)
            } else {
                "END".to_string()
            };
            return Err(PyError::runtime_error("stack underflow (peek)"));
        }
        Ok(self.stack[self.stack.len() - 1 - depth].clone())
    }

    pub fn insert_local(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> {
        self.locals.insert(interner::intern(name), val)
    }

    pub fn get_local(&self, name: &str) -> Option<&PyObjectRef> {
        self.locals.get(interner::intern(name))
    }

    pub fn remove_local(&mut self, name: &str) -> Option<PyObjectRef> {
        self.locals.remove(interner::intern(name))
    }

    pub fn contains_local(&self, name: &str) -> bool {
        self.locals.contains_key(interner::intern(name))
    }
}

pub struct VirtualMachine {
    pub frames: Vec<Frame>,
    pub builtins: Rc<HashMap<StrId, PyObjectRef>>,
    pub modules: HashMap<String, PyObjectRef>,
    pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
    #[cfg(feature = "jit")]
    pub jit: RefCell<JitCompiler>,
    /// Execution profile counters — how many times each instruction ran.
    /// Indexed by (function_id, instruction_offset). Used by JIT to
    /// identify hot paths for native compilation.
    pub profile: RefCell<HashMap<usize, Vec<u32>>>,
    pub frame_pool: Vec<Frame>,
    /// Line number of the last instruction executed. Used for error reporting.
    pub last_error_line: Option<usize>,
    /// Filename of the frame the last instruction executed in. Used for error reporting.
    pub last_error_file: Option<String>,
    /// Call-stack snapshot (filename, line, function name), outermost first,
    /// captured the moment an exception is found to have no handler anywhere
    /// on the current frame stack. Used for top-level traceback printing.
    pub last_traceback: Vec<(String, usize, String)>,
    /// Type registry: maps type names to PyObject::Type objects.
    /// Used by builtin_type_of() to return real type objects instead of strings.
    pub type_registry: HashMap<String, PyObjectRef>,
    /// Current exception info for sys.exc_info()
    pub exc_type: Option<PyObjectRef>,
    pub exc_value: Option<PyObjectRef>,
    pub exc_traceback: Option<PyObjectRef>,
    /// `sys.getrecursionlimit()`/`setrecursionlimit()` — consulted by
    /// `call_function`'s own `self.frames.len()` depth guard (see there for
    /// why this exists at all). Real trigger: CPython's own `test.support.
    /// infinite_recursion(N)` context manager temporarily lowers this to
    /// make deliberately-infinite-recursion tests fail fast instead of
    /// grinding through hundreds of real frames first.
    pub recursion_limit: usize,
}

/// Locate the bundled `Lib/` directory relative to the running executable
/// rather than the current working directory, so the interpreter works when
/// invoked from anywhere (not just the repo root). Walks up from the
/// executable's directory looking for a `Lib` subdirectory (covers both
/// `target/{debug,release}/rustpython` during development and a real
/// install layout), falling back to the old CWD-relative behavior only if
/// that search fails.
fn find_lib_dir() -> String {
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        for _ in 0..5 {
            match dir {
                Some(d) => {
                    let candidate = d.join("Lib");
                    if candidate.is_dir() {
                        return candidate.to_string_lossy().into_owned();
                    }
                    dir = d.parent().map(|p| p.to_path_buf());
                }
                None => break,
            }
        }
    }
    "./Lib".to_string()
}

/// Finds `key`'s slot in `varnames` IF it names a real formal parameter
/// (positional or keyword-only) — NOT just any local variable. `varnames`
/// (CPython's `co_varnames` layout) holds positional params, then kwonly
/// params, then `*args`/`**kwargs` names, then EVERY OTHER plain local the
/// function body ever assigns — a naive `varnames.iter().position(...)`
/// scan over the whole list (the bug this replaced) meant a keyword
/// argument whose name happened to match some unrelated local variable used
/// later in the function body (e.g. `def f(**kwargs): dest =
/// kwargs.pop('dest', None)` called as `f(dest=...)`) got silently
/// misrouted into that local's fast-locals slot instead of `**kwargs`,
/// making it vanish from `kwargs` entirely.
fn formal_param_index(varnames: &[crate::interner::StrId], arg_count: usize, kwonlyarg_count: usize, kwonly_start: usize, key: &str) -> Option<usize> {
    let key_id = crate::interner::intern(key);
    if let Some(idx) = varnames.get(0..arg_count).and_then(|s| s.iter().position(|&n| n == key_id)) {
        return Some(idx);
    }
    if kwonlyarg_count > 0 {
        let end = kwonly_start + kwonlyarg_count;
        if let Some(rel) = varnames.get(kwonly_start..end).and_then(|s| s.iter().position(|&n| n == key_id)) {
            return Some(kwonly_start + rel);
        }
    }
    None
}

/// Parses a single `ConstValue` (the compiler's own, still-textual constant
/// representation — e.g. `ConstValue::Int(String)` holds the LITERAL SOURCE
/// TEXT of an int literal, not a pre-parsed number) into the real
/// `PyObjectRef` a `LOAD_CONST` of it should push. Factored out of
/// `LOAD_CONST`'s own opcode handler so its result can be cached on the
/// `CodeObject` (see `CodeObject::const_cache`'s doc comment) — this
/// function itself is unaware of caching, it's just the (moderately
/// expensive, for `Int`/`Float`/`Complex`) one-time parse.
pub(crate) fn eval_const_value(const_val: ConstValue) -> PyResult<PyObjectRef> {
    Ok(match const_val {
        ConstValue::None => py_none(),
        ConstValue::Bool(b) => py_bool(b),
        ConstValue::Int(s) => {
            // Strips ALL underscores (digit separators, e.g. `1_000_000`),
            // not just leading ones — `try_exec_simple`'s OWN independent
            // copy of this same parsing logic used `s.trim_start_matches
            // ('_')` instead (fixed to match, in the same pass as adding
            // its own const-cache use, since both copies must agree).
            let s_clean: String = s.chars().filter(|&c| c != '_').collect();
            if let Some(oct) = s_clean.strip_prefix("0o").or_else(|| s_clean.strip_prefix("0O")) {
                if let Ok(n) = i64::from_str_radix(oct, 8) { py_int(n) }
                else { let n = BigInt::parse_bytes(oct.as_bytes(), 8).ok_or_else(|| PyError::value_error(format!("invalid integer: {}", s)))?; PyObjectRef::imm(PyObject::Int(n)) }
            } else if let Some(hex) = s_clean.strip_prefix("0x").or_else(|| s_clean.strip_prefix("0X")) {
                if let Ok(n) = i64::from_str_radix(hex, 16) { py_int(n) }
                else { let n = BigInt::parse_bytes(hex.as_bytes(), 16).ok_or_else(|| PyError::value_error(format!("invalid integer: {}", s)))?; PyObjectRef::imm(PyObject::Int(n)) }
            } else if let Some(bin) = s_clean.strip_prefix("0b").or_else(|| s_clean.strip_prefix("0B")) {
                if let Ok(n) = i64::from_str_radix(bin, 2) { py_int(n) }
                else { let n = BigInt::parse_bytes(bin.as_bytes(), 2).ok_or_else(|| PyError::value_error(format!("invalid integer: {}", s)))?; PyObjectRef::imm(PyObject::Int(n)) }
            } else if let Ok(n) = s_clean.parse::<i64>() {
                py_int(n)  // uses small int cache
            } else {
                let n: BigInt = s_clean.parse().map_err(|_| {
                    PyError::value_error(format!("invalid integer: {}", s))
                })?;
                PyObjectRef::imm(PyObject::Int(n))
            }
        }
        ConstValue::Float(s) => {
            let f: f64 = s.parse().map_err(|_| {
                PyError::value_error(format!("invalid float: {}", s))
            })?;
            py_float(f)
        }
        ConstValue::String(s) => py_str(&s),
        ConstValue::Bytes(b) => PyObjectRef::imm(PyObject::Bytes(b)),
        ConstValue::Complex { real, imag } => {
            let re: f64 = real.parse().map_err(|_| {
                PyError::value_error(format!("invalid complex literal: {}", real))
            })?;
            let im: f64 = imag.parse().map_err(|_| {
                PyError::value_error(format!("invalid complex literal: {}", imag))
            })?;
            PyObjectRef::imm(PyObject::Complex(re, im))
        }
        ConstValue::Code(code) => {
            PyObjectRef::imm(PyObject::Code(Rc::from(code)))
        }
        ConstValue::Tuple(items) => {
            let objs: Vec<PyObjectRef> = items.into_iter().map(|s| py_str(&s)).collect();
            PyObjectRef::imm(PyObject::Tuple(objs))
        }
    })
}

impl VirtualMachine {
    pub fn new() -> Self {
        if std::env::var("RPY_DEBUG_VM_NEW").is_ok() {
            eprintln!("VM_NEW");
        }
        Self::new_with_args(std::env::args().collect())
    }

    pub fn new_with_args(argv: Vec<String>) -> Self {
        // Constructing a full VM means running EVERY native module
        // constructor below (~100+ modules) plus filesystem-based venv/
        // site-packages detection — cheap for the ONE real, top-level
        // program VM, but catastrophic for the many *disposable* VMs
        // spun up just to resume one step of a generator/coroutine
        // (`generator_next_fallback`, `coroutine_send_fallback` below —
        // see their own doc comments on why a disposable VM is used at
        // all) since `next()` on a generator does this on EVERY SINGLE
        // resume. Confirmed via a minimal repro: a no-op
        // `@contextlib.contextmanager` (whose `__enter__`/`__exit__` each
        // call `next()` once) entered/exited 14,202 times in a bare loop
        // took 30+ seconds — should be near-instant, and this was the
        // actual root cause behind several CPython test files (test_math,
        // test_statistics, ...) appearing to hang. Real CPython has
        // exactly ONE `sys`/`os`/etc. module per process for its entire
        // lifetime, shared by every generator resume — rebuilding them
        // fresh per resume was never semantically necessary, just an
        // artifact of how disposable-VM construction happened to be
        // written. Caching the built `(builtins, modules)` state after the
        // FIRST construction and Rc-cloning it into every subsequent
        // `new()` call matches that real-CPython "one shared stdlib per
        // process" model directly, and follows the same pattern already
        // established (and proven safe) by `install_source_defined_stdlib`'s
        // own `EXECUTED_STDLIB_CACHE` further below — sharing the actual
        // module/function objects across VM instances doesn't change any
        // observable behavior for ordinary user code (isinstance/identity/
        // method dispatch all keep working, since it's genuinely the same
        // objects everywhere, exactly as real CPython's own process-wide
        // module singletons behave). Safe regardless of `argv`: the ONE
        // real top-level VM (constructed directly with the process's real
        // `sys.argv` in `main.rs`) always runs first and populates this
        // cache; every later disposable VM goes through `new()`, which
        // passes that same real `std::env::args()` anyway.
        thread_local! {
            static VM_STATE_CACHE: RefCell<Option<(Rc<HashMap<StrId, PyObjectRef>>, HashMap<String, PyObjectRef>)>> = RefCell::new(None);
        }
        if let Some((cached_builtins, cached_modules)) = VM_STATE_CACHE.with(|c| c.borrow().clone()) {
            let globals_map: HashMap<StrId, PyObjectRef> = HashMap::from([
                (interner::intern("__name__"), py_str("__main__")),
                (interner::intern("__builtins__"), create_module("builtins", cached_builtins.iter().map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone())).collect::<HashMap<String, PyObjectRef>>())),
            ]);
            let globals = Rc::new(RefCell::new(globals_map));
            let mut vm = VirtualMachine {
                frames: Vec::new(),
                builtins: Rc::clone(&cached_builtins),
                modules: cached_modules,
                globals,
                #[cfg(feature = "jit")]
                jit: RefCell::new(JitCompiler::new()),
                profile: RefCell::new(HashMap::new()),
                last_error_line: None,
                last_error_file: None,
                last_traceback: Vec::new(),
                frame_pool: Vec::new(),
                type_registry: HashMap::new(),
                exc_type: None,
                exc_value: None,
                exc_traceback: None,
                recursion_limit: 1000,
            };
            vm.populate_type_registry();
            vm.install_source_defined_stdlib("collections", crate::modules::COLLECTIONS_USER_TYPES_SOURCE, &["UserList", "UserDict", "UserString", "Counter", "defaultdict", "ChainMap"]);
            vm.install_source_defined_stdlib("functools", crate::modules::FUNCTOOLS_EXTRA_SOURCE, &["lru_cache", "cache"]);
            vm.install_source_defined_stdlib("enum", crate::modules::ENUM_SOURCE, &[
                "auto", "nonmember", "member", "property", "EnumType", "EnumMeta",
                "Enum", "IntEnum", "StrEnum", "unique",
            ]);
            vm.install_source_defined_stdlib("gettext", crate::modules::GETTEXT_SOURCE, &[
                "NullTranslations", "GNUTranslations", "find", "translation", "install",
                "textdomain", "bindtextdomain", "gettext", "ngettext", "pgettext", "npgettext",
                "dgettext", "dngettext", "__all__",
            ]);
            vm.install_source_defined_stdlib("json", crate::modules::JSON_EXTRA_SOURCE, &["JSONEncoder", "dumps"]);
            return vm;
        }

        let builtins_str_map = create_builtins();
        let mut builtins: HashMap<StrId, PyObjectRef> = str_map_to_strid_map(builtins_str_map);
        let builtins_to_module = |map: &HashMap<StrId, PyObjectRef>| {
            map.iter().map(|(k,v)| (interner::lookup_str(*k).to_string(), v.clone())).collect::<HashMap<String, PyObjectRef>>()
        };
        let globals_map: HashMap<StrId, PyObjectRef> = HashMap::from([
            (interner::intern("__name__"), py_str("__main__")),
            (interner::intern("__builtins__"), create_module("builtins", builtins_to_module(&builtins))),
        ]);
        let globals = Rc::new(RefCell::new(globals_map));

         let mut modules: HashMap<String, PyObjectRef> = HashMap::new();
         modules.insert_str("builtins", create_module("builtins", builtins_to_module(&builtins)));
         modules.insert_str("math", create_module("math", create_math_dict()));
         modules.insert_str("_codecs", create_module("_codecs", create_codecs_dict()));

         let mut sys_dict = create_sys_dict(argv);
         // sys.path is shared (Rc-cloned) across every VirtualMachine
         // instance in this process/thread, real or disposable — see the
         // populate-defaults block below for why. `sys.modules` is
         // deliberately NOT shared this way: it's populated per-VM below
         // from that VM's own already-built `modules` map, which is the
         // correct, VM-local behavior for module caching.
         thread_local! {
             static SHARED_SYS_PATH: RefCell<Option<PyObjectRef>> = RefCell::new(None);
         }
         let reused_shared_path = SHARED_SYS_PATH.with(|c| c.borrow().clone());
         if let Some(shared_path) = reused_shared_path.clone() {
             sys_dict.insert_str("path", shared_path);
         }
         // sys.meta_path — import hooks
         if !sys_dict.contains_key("meta_path") {
             let meta_path = py_list(vec![
                 PyObjectRef::imm(PyObject::BuiltinFunction {
                     name: "BuiltinImporter".to_string(),
                     func: |args| {
                         if args.len() < 2 { return Err(PyError::type_error("find_spec() requires 2 arguments")); }
                         Err(PyError::module_not_found_error(format!("No module named '{}'", args[1].str())))
                     },
                 }),
             ]);
             sys_dict.insert_str("meta_path", meta_path);
         }
         if !sys_dict.contains_key("path_hooks") {
             sys_dict.insert_str("path_hooks", py_list(vec![]));
         }
         if !sys_dict.contains_key("path_importer_cache") {
             sys_dict.insert_str("path_importer_cache", py_dict());
         }
          modules.insert_str("sys", create_module("sys", sys_dict.clone()));
           for (k, v) in sys_dict.clone() { builtins.insert(interner::intern(&k), v); }

         // Native os module
         let os_mod = create_module("os", create_os_dict());
         modules.insert_str("os", os_mod.clone());
         // posix is the C extension behind os — alias it for importlib compatibility
         modules.insert_str("posix", os_mod.clone());

         // Native os.path submodule (path manipulation functions)
         let os_path_mod = create_module("os.path", create_os_path_dict());
         // Wire path as a submodule attribute of the os parent module
         if let PyObject::Module { dict, .. } = &mut *os_mod.borrow_mut() {
             dict.insert_str("path", os_path_mod.clone());
         }
         modules.insert_str("os.path", os_path_mod.clone());
         // posixpath is the real module behind os.path on POSIX (CPython's
         // own os.py does `sys.modules['os.path'] = posixpath`) — code that
         // imports it directly (`import posixpath`, common in stdlib-ish
         // path-handling helpers) expects the same functions os.path has.
         modules.insert_str("posixpath", os_path_mod);

         let pathlib_dict = create_pathlib_dict();
         modules.insert_str("pathlib", create_module("pathlib", pathlib_dict));

         // Native urllib package (urllib.request, urllib.parse)
         let urllib_dict = create_urllib_dict();
         modules.insert_str("urllib", create_module("urllib", urllib_dict));

         let json_dict = create_json_dict();
         modules.insert_str("json", create_module("json", json_dict));

         let collections_dict = create_collections_dict(builtins.get(&interner::intern("object")).cloned().unwrap_or_else(py_none));
         modules.insert_str("collections", create_module("collections", collections_dict));

          let functools_dict = create_functools_dict();
          modules.insert_str("functools", create_module("functools", functools_dict));

          let itertools_dict = create_itertools_dict();
          modules.insert_str("itertools", create_module("itertools", itertools_dict));


          let datetime_dict = create_datetime_dict();
          modules.insert_str("datetime", create_module("datetime", datetime_dict));
          // `_datetime` is real CPython's C-accelerated backing module —
          // `datetime.py` itself does `from _datetime import *` when
          // available. This interpreter's `datetime` is already a single,
          // natively-implemented module (no separate accelerated/pure-
          // Python split), so `_datetime` is just an alias — needed only so
          // code that imports `_datetime` directly (real trigger: CPython's
          // own `test_module.py`-style introspection, checking that both
          // names resolve) doesn't raise `ImportError`.
          modules.insert_str("_datetime", create_module("_datetime", create_datetime_dict()));

          let zoneinfo_dict = create_zoneinfo_dict();
          modules.insert_str("zoneinfo", create_module("zoneinfo", zoneinfo_dict));

          let socket_dict = create_socket_dict();
          modules.insert_str("socket", create_module("socket", socket_dict.clone()));
          modules.insert_str("_socket", create_module("_socket", socket_dict));

          let select_dict = create_select_dict();
          modules.insert_str("select", create_module("select", select_dict));

          let re_dict = create_re_dict();
          modules.insert_str("re", create_module("re", re_dict));

          let subprocess_dict = create_subprocess_dict();
          modules.insert_str("subprocess", create_module("subprocess", subprocess_dict));

          // Native pickle module (basic stub)
          modules.insert_str("_pickle", create_module("_pickle", create_pickle_dict()));

          // Native logging module
          modules.insert_str("_logging", create_module("_logging", create_logging_dict()));
          modules.insert_str("_logging.config", create_module("_logging.config", create_logging_config_dict()));

          // Native timeit module
          modules.insert_str("timeit", create_module("timeit", create_timeit_dict()));

          let threading_dict = create_threading_dict();
          modules.insert_str("threading", create_module("threading", threading_dict));

          // Native _thread module (CPython C extension replacement)
          modules.insert_str("_thread", create_module("_thread", create_thread_module_dict()));

          // Native signal module (CPython C extension replacement)
          modules.insert_str("signal", create_module("signal", create_signal_dict()));

          // Native gc module (CPython C extension replacement)
          modules.insert_str("gc", create_module("gc", create_gc_dict()));

          // Native sysconfig module (CPython stdlib replacement)
          modules.insert_str("sysconfig", create_module("sysconfig", create_sysconfig_dict()));

          // Native linecache module (CPython stdlib replacement)
          modules.insert_str("linecache", create_module("linecache", create_linecache_dict()));

          // Native calendar module
          modules.insert_str("calendar", create_module("calendar", create_calendar_dict()));

          // Native locale module
          modules.insert_str("locale", create_module("locale", create_locale_dict()));
          // Native _locale module — in real CPython this IS the C extension
          // that the pure-Python `locale` module delegates to. Registering it
          // natively (instead of the Lib/_locale.py stub) makes `setlocale`,
          // `localeconv`, etc. real shared state (see create_locale_dict).
          modules.insert_str("_locale", create_module("_locale", create_locale_dict()));

          // gettext module (mostly Python source — see install_source_defined_stdlib below)
          modules.insert_str("gettext", create_module("gettext", create_gettext_dict()));

          // Native ssl module (CPython C extension replacement for urllib3 compatibility)
          modules.insert_str("ssl", create_module("ssl", create_ssl_dict()));

          // Native time module
          modules.insert_str("time", create_module("time", create_time_dict()));

          // Native C extension replacements for CPython stdlib compatibility
          let weakref_dict = create_weakref_dict();
          modules.insert_str("_weakref", create_module("_weakref", weakref_dict.clone()));

          let collections_abc_dict = create_collections_abc_dict();
          modules.insert_str("_collections_abc", create_module("_collections_abc", collections_abc_dict.clone()));
          // Pre-register collections.abc so the import chain walker finds it without needing __path__
          modules.insert_str("collections.abc", create_module("collections.abc", collections_abc_dict));

          // Native weakref module (replaces CPython weakref.py)
          let mut weakref_mod_dict = weakref_dict; // Start from _weakref
          // Add WeakValueDictionary and WeakKeyDictionary as dict-like stubs
          weakref_mod_dict.insert_str("WeakValueDictionary", create_weakref_weak_val_dict());
          weakref_mod_dict.insert_str("WeakKeyDictionary", create_weakref_weak_key_dict());
          weakref_mod_dict.insert_str("WeakSet", create_weakref_weak_set());
          modules.insert_str("weakref", create_module("weakref", weakref_mod_dict));

          // Native copy module (replaces CPython copy.py which uses unsupported syntax)
          modules.insert_str("copy", create_module("copy", create_copy_dict()));

          // Native types module (replaces CPython types.py)
          modules.insert_str("types", create_module("types", create_types_dict()));

          // Native struct module for binary packing
          modules.insert_str("struct", create_module("struct", create_struct_dict()));

          // Native bisect module for binary search
          modules.insert_str("bisect", create_module("bisect", create_bisect_dict()));

          // Native heapq module for heap queue operations
          modules.insert_str("heapq", create_module("heapq", create_heapq_dict()));

          // enum module — real Enum/IntEnum/StrEnum/EnumType semantics
          // (metaclass, real members, auto/unique) are far easier and more
          // correct expressed as real Python source (see enum_extra.py)
          // than as hand-written Rust closures; install_source_defined_stdlib
          // (called below, once builtins/type registry exist) fills this
          // module's dict in. The empty dict here is just a placeholder
          // registration so that call finds an existing module to populate.
          modules.insert_str("enum", create_module("enum", HashMap::new()));

          // Native glob module
          modules.insert_str("glob", create_module("glob", create_glob_dict()));

          // Native fnmatch module
          modules.insert_str("fnmatch", create_module("fnmatch", create_fnmatch_dict()));

          // Native textwrap module
          modules.insert_str("textwrap", create_module("textwrap", create_textwrap_dict()));

          // Native pprint module
          modules.insert_str("pprint", create_module("pprint", create_pprint_dict()));

          // Native hashlib module
          modules.insert_str("hashlib", create_module("hashlib", create_hashlib_dict()));

          // Native secrets module
          modules.insert_str("secrets", create_module("secrets", create_secrets_dict()));

          // Native hmac module
          modules.insert_str("hmac", create_module("hmac", create_hmac_dict()));

          // Native base64 module
          modules.insert_str("base64", create_module("base64", create_base64_dict()));

          // Native binascii module
          modules.insert_str("binascii", create_module("binascii", create_binascii_dict()));

          // Native uuid module
          modules.insert_str("uuid", create_module("uuid", create_uuid_dict()));

          // Native string module (with capwords and Formatter)
          let mut string_dict = create_string_dict();
          let string_v2 = create_string_dict_v2();
          for (k, v) in string_v2 { string_dict.insert(k, v); }
          modules.insert_str("string", create_module("string", string_dict));

          // Native colorsys module
          modules.insert_str("colorsys", create_module("colorsys", create_colorsys_dict()));

          // Native wave module
          modules.insert_str("wave", create_module("wave", create_wave_dict()));

          // Native numbers module — DISABLED: was bare STRING placeholders
          // (`d.insert_str("Number", py_str("Number"))` etc.) instead of
          // real ABC classes — `isinstance(x, numbers.Number)` may have
          // worked via some ad hoc string-matching path, but anything
          // doing real class things with them (`numbers.Number.register
          // (Decimal)` — needed by `decimal`'s own real implementation,
          // see its own doc comment above) raised `AttributeError: 'str'
          // object has no attribute 'register'`. Real CPython's `numbers.
          // py` is small (427 lines) and pure Python (just `abc.ABCMeta`/
          // `abstractmethod`) — vendored verbatim instead, same pattern as
          // `decimal`/`html.parser` above. `create_numbers_dict` (modules/
          // misc.rs) is now dead code, kept only in case `Lib/numbers.py`
          // needs to be reverted.
          // modules.insert_str("numbers", create_module("numbers", create_numbers_dict()));

          // `ast` now loads from Lib/ast.py — needs real (if minimal, marker-
          // only) node classes for PEP 649 lazy-annotation stringification
          // (`annotationlib.py`'s `_Stringifier`, needed transitively by
          // `test.support`), which the old native stub (just `literal_eval`
          // plus a handful of node NAMES as bare strings, no real classes at
          // all) couldn't provide. The old stub's actual `literal_eval`
          // logic is kept and re-exposed under a private native module name
          // so Lib/ast.py can still delegate to it instead of reimplementing
          // literal parsing in pure Python.
          modules.insert_str("_ast_native", create_module("_ast_native", create_ast_dict()));

          // Native sunau module (Sun AU audio format stubs)
          modules.insert_str("sunau", create_module("sunau", create_sunau_dict()));

          // Native difflib module (with unified_diff)
          modules.insert_str("difflib", create_module("difflib", create_difflib_dict()));

          // Native csv module
          modules.insert_str("csv", create_module("csv", create_csv_dict()));

          // Native io module — DISABLED: CPython io.py is used instead (imports from _io)
          // modules.insert_str("io", create_module("io", create_io_dict()));

          // Native statistics module. Tried vendoring the real CPython
          // `Lib/statistics.py` (same pattern as `html`/`numbers`/etc.) —
          // reverted: it hits the same unresolved "native types aren't real
          // Type objects" architecture gap as `decimal` (`type(5) is int`
          // is `False` here, breaking `_coerce`'s `T is S`/`T is int`
          // identity checks throughout `_sum`/`variance`/etc.), AND
          // `test_statistics.py` timed out (60s+) rather than completing —
          // an operational risk not worth taking for a module that would
          // still fail most of its own tests anyway. Several genuinely
          // general bugs found chasing this ARE kept (not reverted):
          // `functools.reduce`'s dropped-initial-value bug, `math.fsum`
          // only handling List/Tuple, `itertools.groupby` (was missing
          // entirely), `int.as_integer_ratio`/`numerator`/`denominator`/
          // `real`/`imag`, `call_bound_method` supporting `type` as a
          // plain callable, and — the most impactful one — `WITH_EXIT`
          // never recognizing a user-defined exception CLASS instance
          // (only the native `PyObject::Exception` shape), which broke
          // `with`-statement `__exit__(exc_type, exc_value, tb)` for any
          // custom exception (`unittest.assertRaises` reported ANY custom
          // exception as "not raised" even when it genuinely was).
          modules.insert_str("statistics", create_module("statistics", create_statistics_dict()));

          // Native contextlib module — DISABLED: real Lib/contextlib.py is used instead
          // modules.insert_str("contextlib", create_module("contextlib", create_contextlib_dict()));

          // Native decimal module. (Attempted vendoring real CPython's
          // `decimal.py`/`_pydecimal.py` this session — got as far as
          // `numbers.Number.register(Decimal)` working [needed the
          // `numbers` vendor + a new generic `.register()`/`isinstance`-
          // registry fallback, both kept, see below] before hitting
          // `int.bit_length` accessed UNBOUND off the `int` type itself
          // (`_nbits = int.bit_length`) — `int` is a `PyObject::
          // BuiltinFunction`, not a real `Type`, so it has no attribute
          // lookup for "what method would an instance's `.bit_length()`
          // resolve to" at all. That's a deeper, general "unbound method
          // access on a native-backed type" gap shared with list/dict/str/
          // etc., not specific to decimal — reverted the vendor rather
          // than chase it further this session. `Lib/decimal.py`/`Lib/
          // _pydecimal.py` were removed again; re-attempt once unbound
          // native-type method access is fixed.)
          modules.insert_str("decimal", create_module("decimal", create_decimal_dict()));

          // Native fractions module
          modules.insert_str("fractions", create_module("fractions", create_fractions_dict()));

          // Native platform module
          modules.insert_str("platform", create_module("platform", create_platform_dict()));

          // Native getopt module
          modules.insert_str("getopt", create_module("getopt", create_getopt_dict()));

          // Native getpass module
          modules.insert_str("getpass", create_module("getpass", create_getpass_dict()));

          // Native errno module
          modules.insert_str("errno", create_module("errno", create_errno_dict()));

          // Native _random module (C extension stub for CPython's random.py)
          modules.insert_str("_random", create_module("_random", create_random_cmodule_dict()));

          // Native shutil module
          modules.insert_str("shutil", create_module("shutil", create_shutil_dict()));

          // Native graphlib module
          modules.insert_str("graphlib", create_module("graphlib", create_graphlib_dict()));

          // Native pdb module
          modules.insert_str("pdb", create_module("pdb", create_pdb_dict()));

          // traceback now loads from Lib/traceback.py — the old native stub
          // (`create_traceback_dict`, kept as dead code) had only
          // `format_exc`/`print_exc` as no-ops and no `TracebackException`
          // at all, which real `unittest/result.py` needs to format a
          // failure/error for display.
          // modules.insert_str("traceback", create_module("traceback", create_traceback_dict()));

          // Native warnings module
          modules.insert_str("warnings", create_module("warnings", create_warnings_dict()));

          // Native abc module
          modules.insert_str("abc", create_module("abc", create_abc_dict()));

          // `_py_abc` — real CPython's separate pure-Python reference
          // implementation of `ABCMeta` (used by `test_abc.py`'s own
          // `test_factory(abc.ABCMeta, ...)` / `test_factory(_py_abc.ABCMeta,
          // ...)` pattern to run its whole suite against both the C and
          // Python implementations). This codebase has only ONE `ABCMeta`
          // implementation (native Rust, no separate "C vs Python" split at
          // all), so `_py_abc` was missing entirely — `import _py_abc`
          // failed outright, crashing `test_abc.py` at collection before a
          // single test ran. Aliased to the exact same dict as `abc` itself:
          // not a literal from-scratch Python reimplementation, but an
          // honest match for this codebase's actual architecture — it
          // unblocks the import and lets both `test_factory` calls exercise
          // real, working `ABCMeta` functionality (just the same
          // implementation twice under two names, rather than two distinct
          // ones).
          modules.insert_str("_py_abc", create_module("_py_abc", create_abc_dict()));

          // Native typing module (type annotation stubs)
          // Comment out native typing - use Lib/typing.py instead
          // modules.insert_str("typing", create_module("typing", create_typing_dict()));

          // Native pickle module
          modules.insert_str("_pickle", create_module("_pickle", create_pickle_dict()));

          // Native logging module
          modules.insert_str("_logging", create_module("_logging", create_logging_dict()));

          // Native timeit module
          modules.insert_str("timeit", create_module("timeit", create_timeit_dict()));

          // Native json.tool module
          modules.insert_str("json.tool", create_module("json.tool", create_json_tool_dict()));

          // Native cmath module (complex math: sqrt, sin, cos)
          modules.insert_str("cmath", create_module("cmath", create_cmath_dict()));

          // Native gzip module
          modules.insert_str("gzip", create_module("gzip", create_gzip_dict()));

          // Native zlib module
          modules.insert_str("zlib", create_module("zlib", create_zlib_dict()));

          // Native tarfile module
          modules.insert_str("tarfile", create_module("tarfile", create_tarfile_dict()));

          // Native zipfile module (read-only)
          modules.insert_str("zipfile", create_module("zipfile", create_zipfile_dict()));

          // Native hashlib_extra module
          modules.insert_str("hashlib_extra", create_module("hashlib_extra", create_hashlib_extra_dict()));

          // dataclasses now loads from Lib/dataclasses.py (a real, if
          // simplified, implementation — field generation, generated
          // __init__/__repr__/__eq__, __dataclass_fields__, fields(), etc.)
          // instead of this native stub, which only ever tagged classes with
          // a marker attribute and never generated anything.
          // modules.insert_str("dataclasses", create_module("dataclasses", create_dataclasses_dict()));

          // Native operator module
          modules.insert_str("operator", create_module("operator", create_operator_dict()));
          // `_operator` — real CPython's C-accelerated backing module for
          // `operator.py` (`from _operator import *`); same alias rationale
          // as `_datetime` above.
          modules.insert_str("_operator", create_module("_operator", create_operator_dict()));

          // Native reprlib module
          modules.insert_str("reprlib", create_module("reprlib", create_reprlib_dict()));

          // Native array module
          modules.insert_str("array", create_module("array", create_array_dict()));

          // Native shelve module (persistent dict wrapper)
          modules.insert_str("shelve", create_module("shelve", create_shelve_dict()));

          // Native mimetypes module
          modules.insert_str("mimetypes", create_module("mimetypes", create_mimetypes_dict()));

          // Native dis module for bytecode disassembly
          modules.insert_str("dis", create_module("dis", create_dis_dict()));

          // Native http module (HTTPStatus enum)
          let http_mod = create_module("http", create_http_dict());
          modules.insert_str("http", http_mod.clone());

          // Native http.client submodule (HTTPConnection, HTTPResponse)
          let http_client_mod = create_module("http.client", create_http_client_dict());
          // Wire client as a submodule attribute of the http parent module
          if let PyObject::Module { dict, .. } = &mut *http_mod.borrow_mut() {
              dict.insert_str("client", http_client_mod.clone());
          }
          modules.insert_str("http.client", http_client_mod);

          // Native smtplib module (SMTP stub)
          modules.insert_str("smtplib", create_module("smtplib", create_smtplib_dict()));

          // Native html/html.entities/html.parser — DISABLED: `html.parser`
          // was a near-empty stub (no real tokenizer at all — `feed()` just
          // accumulated raw text verbatim, none of `handle_starttag`/
          // `handle_endtag`/`handle_data`/etc. were ever called), and built
          // as a `PyObject::BuiltinFunction` rather than a real
          // `PyObject::Type`, so `class EventCollector(html.parser.
          // HTMLParser): ...` (real subclassing, overriding those handler
          // methods — CPython's own `test_htmlparser.py`'s entire approach)
          // couldn't inherit anything from it at all (`AttributeError:
          // 'EventCollector' object has no attribute 'feed'`). Real
          // CPython's `html`/`html.entities`/`html.parser` are pure Python
          // (plus `_markupbase`, `html.parser`'s shared tokenizer-support
          // base) — vendored verbatim from a real CPython 3.14 install
          // rather than reimplemented, same "vendor as pure-Python Lib/
          // module" pattern as `unittest`/`doctest`/`email` above. Resolved
          // through the normal file-based import path instead —
          // `Lib/html/__init__.py` (escape/unescape), `Lib/html/
          // entities.py` (html5/name2codepoint/codepoint2name data),
          // `Lib/html/parser.py` (the real HTMLParser), `Lib/
          // _markupbase.py`. `create_html_dict`/`create_html_entities_dict`/
          // `create_html_parser_dict` (modules/net.rs, modules/text.rs) are
          // now dead code, kept only in case `Lib/html/` needs to be
          // reverted.
          // let html_mod = create_module("html", create_html_dict());
          // modules.insert_str("html", html_mod.clone());
          // let html_entities_mod = create_module("html.entities", create_html_entities_dict());
          // if let PyObject::Module { dict, .. } = &mut *html_mod.borrow_mut() {
          //     dict.insert_str("entities", html_entities_mod.clone());
          // }
          // modules.insert_str("html.entities", html_entities_mod);
          // let html_parser_mod = create_module("html.parser", create_html_parser_dict());
          // if let PyObject::Module { dict, .. } = &mut *html_mod.borrow_mut() {
          //     dict.insert_str("parser", html_parser_mod.clone());
          // }
          // modules.insert_str("html.parser", html_parser_mod);

          // Native unittest module — DISABLED: was a complete no-op stub
          // (every assertX method silently did nothing, `main()` never
          // discovered or ran a single test) — replaced with the real
          // CPython pure-Python `unittest` package (Lib/unittest/). Real
          // CPython/Django test suites are unittest-based; silently
          // no-op'ing every assertion is actively dangerous for a project
          // whose goal is being a genuine CPython replacement.
          // modules.insert_str("unittest", create_module("unittest", create_unittest_dict()));

          // Native doctest module used to be a hollow stub (testmod/testfile
          // always reported 0 attempted/0 failed regardless of actual
          // docstring content; DocTestSuite/DocFileSuite didn't exist at all
          // — real trigger: 16+ CPython test files' own `load_tests` doing
          // `tests.addTest(doctest.DocTestSuite())`, crashing with
          // `AttributeError: 'module' object has no attribute
          // 'DocTestSuite'`). Replaced with a real (if simplified) Python
          // implementation at `Lib/doctest.py`, resolved through the normal
          // file-based import path instead — same "vendor/reimplement as a
          // pure-Python Lib/ module" pattern as `unittest`/`email`.
          // modules.insert_str("doctest", create_module("doctest", create_doctest_dict()));

          // `email` used to be a thin native stub (EmailMessage/MIMEText/
          // header/utils only, no real Message class, no submodule files —
          // couldn't satisfy `import email.message`/`email.mime.multipart`/
          // etc. at all). Real CPython's own `email` package is pure
          // Python and self-contained; a full copy now lives at
          // `Lib/email/` (plus `Lib/quopri.py`, one of its few deps) and is
          // resolved through the normal file-based import path instead —
          // no native registration needed anymore. `create_email_dict`/
          // `create_email_mime_text_dict`/`create_email_header_dict`/
          // `create_email_utils_dict` (modules/misc.rs) are now dead code,
          // kept only in case `Lib/email/` needs to be reverted.

          // Native configparser module
          modules.insert_str("configparser", create_module("configparser", create_configparser_dict()));

          // Native xml.etree.ElementTree module
          let xml_etree_mod = create_module("xml.etree.ElementTree", create_xml_etree_dict());
          modules.insert_str("xml.etree.ElementTree", xml_etree_mod.clone());
          // `xml.etree` (the bare PACKAGE, distinct from its
          // `.ElementTree` submodule) previously had no entry of its own in
          // `vm.modules` at all — only the leaf `xml.etree.ElementTree` was
          // registered — so `import xml.etree` (without the submodule
          // suffix, a real, common form — real trigger: several CPython
          // corpus files) raised `ImportError: No module named 'xml.etree'`
          // even though the deeper `xml.etree.ElementTree` import worked
          // fine. Fixed by registering the package itself too, with
          // `ElementTree` wired as its own attribute (mirroring the
          // existing `xml`-package-wires-`etree` pattern just below).
          let xml_etree_pkg = create_module("xml.etree", HashMap::new());
          if let PyObject::Module { dict: xml_etree_pkg_dict, .. } = &mut *xml_etree_pkg.borrow_mut() {
              xml_etree_pkg_dict.insert_str("ElementTree", xml_etree_mod.clone());
          }
          modules.insert_str("xml.etree", xml_etree_pkg.clone());
          // Native xml module (empty package)
          let xml_mod = create_module("xml", create_xml_dict());
          // Wire etree as a submodule of xml
          if let PyObject::Module { dict: xml_el_dict, .. } = &mut *xml_mod.borrow_mut() {
              xml_el_dict.insert_str("etree", xml_etree_pkg.clone());
          }
          modules.insert_str("xml", xml_mod);

          // Native this module (Zen of Python)
          modules.insert_str("this", create_module("this", create_this_dict()));

          // argparse now loads from Lib/argparse.py (real CPython source,
          // vendored verbatim) instead of the old native stub — the stub's
          // `add_argument` was a no-op and `parse_args` never populated a
          // caller-supplied `namespace` object (2nd positional arg), which
          // is exactly the calling convention `unittest.main()`'s own
          // `TestProgram.parseArgs` and Django's management-command
          // machinery both rely on. See `create_argparse_dict` (kept, now
          // dead code) for the old implementation.
          // modules.insert_str("argparse", create_module("argparse", create_argparse_dict()));

          // Native _imp module (CPython C extension replacement needed by importlib._bootstrap)
          modules.insert_str("_imp", create_module("_imp", create_imp_dict()));
          // Native _opcode module (needed by test.support)
          modules.insert_str("_opcode", create_module("_opcode", create_opcode_dict()));
          // Native _warnings module (CPython C extension replacement)
          modules.insert_str("_warnings", create_module("_warnings", create_warnings_c_dict()));
          // Native marshal module (CPython C extension replacement)
          modules.insert_str("marshal", create_module("marshal", create_marshal_dict()));
          // Native zipimport module stub
          modules.insert_str("zipimport", create_module("zipimport", create_zipimport_dict()));
          // Native _io module (CPython C extension replacement needed by importlib._bootstrap_external)
          modules.insert_str("_io", create_module("_io", create_io_module_dict()));
          // Native queue module (Queue backed by PyObject::Queue)
          modules.insert_str("queue", create_module("queue", create_queue_dict()));

          // Native importlib stub module
          let importlib_mod = create_module("importlib", create_importlib_dict());
          // Wire importlib.resources as a submodule
          {
              let resources_mod = create_module("importlib.resources", create_importlib_resources_dict());
              if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                  dict.insert_str("resources", resources_mod.clone());
              }
              modules.insert_str("importlib.resources", resources_mod);
          }
          // Wire importlib.util as a submodule
          {
              let util_mod = create_module("importlib.util", create_importlib_util_dict());
              if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                  dict.insert_str("util", util_mod.clone());
              }
              modules.insert_str("importlib.util", util_mod);
          }
          // Add __path__ so dotted imports like importlib.machinery can find filesystem submodules
          {
              if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                  dict.insert_str("__path__", py_list(vec![py_str(&format!("{}/importlib", find_lib_dir()))]));
              }
          }
          modules.insert_str("importlib", importlib_mod);

          modules.insert_str("inspect", create_module("inspect", create_inspect_dict()));

          // Native __future__ module (needed by requests, etc.)
          modules.insert_str("__future__", create_module("__future__", create_future_dict()));

          // Native asyncio module (basic event loop)
          modules.insert_str("asyncio", create_module("asyncio", create_asyncio_dict()));

          // Native atexit module (register/unregister exit callbacks)
          modules.insert_str("atexit", create_module("atexit", create_atexit_dict()));

          // Native contextvars module (ContextVar with thread-local storage)
          modules.insert_str("contextvars", create_module("contextvars", create_contextvars_dict()));

          // Native unicodedata module (basic Unicode category/normalize)
          modules.insert_str("unicodedata", create_module("unicodedata", create_unicodedata_dict()));

          // Native profile module
          modules.insert_str("profile", create_module("profile", create_profile_dict()));

          // Native cProfile module
          modules.insert_str("cProfile", create_module("cProfile", create_cprofile_dict()));

          // Native resource module (POSIX resource usage stubs)
          modules.insert_str("resource", create_module("resource", create_resource_dict()));

          // Native trace module (code tracing / coverage stubs)
          modules.insert_str("trace", create_module("trace", create_trace_dict()));

          // Native _concurrent module (concurrent.futures backend)
          let concurrent_futures_mod = create_module("concurrent.futures", create_concurrent_futures_dict());
          // Create intermediate concurrent package and wire futures under it
          let concurrent_mod = create_module("concurrent", HashMap::new());
          {
              let mut conc_mut = concurrent_mod.borrow_mut();
              if let PyObject::Module { dict, .. } = &mut *conc_mut {
                  dict.insert_str("futures", concurrent_futures_mod.clone());
              }
          }
          modules.insert_str("concurrent", concurrent_mod);
          modules.insert_str("concurrent.futures", concurrent_futures_mod);

          // Native sqlite3 module (requires --features sqlite3)
          #[cfg(feature = "sqlite3")]
          {
              let sqlite3_mod = create_module("sqlite3", create_sqlite3_dict());
              modules.insert_str("sqlite3", sqlite3_mod.clone());
              // sqlite3.dbapi2 — real CPython's sqlite3 package re-exports
              // everything under this name too (the legacy PEP 249 DB-API
              // 2.0 module alias). Real code: Django's own
              // `django/db/backends/sqlite3/base.py` does `from sqlite3
              // import dbapi2 as Database`. Same module object, not a
              // separate copy — matches how CPython's own `dbapi2.py` is
              // just `from sqlite3.dbapi2 import *`-equivalent re-exports.
              modules.insert_str("sqlite3.dbapi2", sqlite3_mod);
          }

          // Populate sys.path with default search paths — ONLY the first
          // time this process/thread creates a VM. Every subsequent
          // VirtualMachine::new() (crucially including the many *disposable*
          // ones `call_bound_method` spins up per nested Python-level call —
          // a separate, documented architectural gap) reuses the exact same
          // list object via `reused_shared_path` above instead of rebuilding
          // it from scratch. This fixes a real correctness bug (a disposable
          // VM's sys.path previously reverted to just these hardcoded
          // defaults, invisible to whatever `sys.path.insert(...)` the
          // running script had done against the real VM — breaking any
          // nested call that needed to import a real site-packages module,
          // confirmed via Django's `django.db.utils.load_backend`) and a
          // real performance win (this block does venv detection + `.pth`
          // file filesystem I/O, previously repeated on every one of 2000+
          // disposable VMs in one observed Django import).
        if reused_shared_path.is_none() {
        if let PyObject::List(path_list) = &mut *sys_dict.get("path").unwrap().borrow_mut() {
            path_list.push(py_str("."));
            path_list.push(py_str(&find_lib_dir()));

            // Read PYTHONPATH environment variable
            if let Ok(pythonpath) = std::env::var("PYTHONPATH") {
                for p in pythonpath.split(':') {
                    let trimmed = p.trim();
                    if !trimmed.is_empty() {
                        path_list.push(py_str(trimmed));
                    }
                }
            }

            // Detect virtual environment (VIRTUAL_ENV, conda, poetry, pixi, or .venv in CWD)
            let venv = std::env::var("VIRTUAL_ENV").ok()
                .or_else(|| std::env::var("CONDA_PREFIX").ok())
                .or_else(|| {
                    if std::env::var("POETRY_ACTIVE").is_ok() {
                        std::env::var("POETRY_VIRTUAL_ENV").ok()
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    std::env::var("PIXI_IN_SHELL").ok().and_then(|_| std::env::var("PIXI_PROJECT_ROOT").ok())
                })
                .or_else(|| {
                    let cwd = std::env::current_dir().ok();
                    if cfg!(feature = "profile") { eprintln!("DEBUG venv: VIRTUAL_ENV not set, checking CWD .venv"); }
                    if let Some(ref d) = cwd {
                        let dotvenv = d.join(".venv");
                        if cfg!(feature = "profile") { eprintln!("DEBUG venv: checking {}. is_dir={}", dotvenv.display(), dotvenv.is_dir()); }
                    }
                    cwd
                        .filter(|d| d.join(".venv").is_dir())
                        .map(|d| d.join(".venv").to_string_lossy().to_string())
                });

            if let Some(ref venv_path) = venv {
                // Try to read pyvenv.cfg to determine the Python version
                let py_version = std::fs::read_to_string(format!("{}/pyvenv.cfg", venv_path))
                    .ok()
                    .and_then(|cfg| {
                        for line in cfg.lines() {
                            if let Some(ver) = line.strip_prefix("version = ") {
                                // Parse "3.13.2" -> "3.13"
                                let parts: Vec<&str> = ver.splitn(2, '.').collect();
                                if parts.len() == 2 {
                                    let major_minor = if let Some(dot2) = parts[1].find('.') {
                                        &parts[1][..dot2]
                                    } else {
                                        parts[1]
                                    };
                                    return Some(format!("{}.{}", parts[0], major_minor));
                                }
                            }
                        }
                        None
                    })
                    .unwrap_or_else(|| "3.13".to_string());

                // Add site-packages directory
                let site_pkg = format!("{}/lib/python{}/site-packages", venv_path, py_version);
                if std::path::Path::new(&site_pkg).is_dir() {
                    path_list.push(py_str(&site_pkg));

                    // Process .pth files in site-packages (e.g., easy-install.pth, distutils-precedence.pth)
                    if let Ok(entries) = std::fs::read_dir(&site_pkg) {
                        for entry in entries.flatten() {
                            let entry_path = entry.path();
                            if entry_path.extension().map_or(false, |e| e == "pth") {
                                if let Ok(content) = std::fs::read_to_string(&entry_path) {
                                    for line in content.lines() {
                                        let trimmed = line.trim();
                                        if trimmed.is_empty() || trimmed.starts_with('#') {
                                            continue;
                                        }
                                        if trimmed.starts_with('.') || trimmed.starts_with('/') {
                                            let resolved = if trimmed.starts_with('.') {
                                                format!("{}/{}", site_pkg, trimmed)
                                            } else {
                                                trimmed.to_string()
                                            };
                                            if !path_list.iter().any(|p| {
                                                p.borrow().str() == resolved
                                            }) {
                                                path_list.push(py_str(&resolved));
                                            }
                                        }
                                        // 'import' directives in .pth are skipped for now
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        }
        // First VM in this process/thread: publish the freshly-populated
        // path list so every subsequent VirtualMachine::new() reuses it.
        if reused_shared_path.is_none() {
            let path_list = sys_dict.get("path").unwrap().clone();
            SHARED_SYS_PATH.with(|c| *c.borrow_mut() = Some(path_list));
        }
         // Populate sys.modules with already-loaded modules
         if let PyObject::Dict(mod_dict) = &mut *sys_dict.get("modules").unwrap().borrow_mut() {
             for (name, module) in &modules {
                 mod_dict.set(py_str(name), module.clone()).ok();
             }
         }

          // Wrap builtins in Rc for sharing across frames
          let builtins = Rc::new(builtins);

          // Populate the disposable-VM fast path's cache (see the doc
          // comment at the top of this function) — safe to do BEFORE the
          // `install_source_defined_stdlib` calls below even though those
          // still mutate module dicts afterward: this clones the `modules`
          // HashMap's *entries* (Rc-bumps), not the module objects
          // themselves, so any mutation those calls make lands on the same
          // shared objects this cache already points to.
          VM_STATE_CACHE.with(|c| *c.borrow_mut() = Some((Rc::clone(&builtins), modules.clone())));

          let mut vm = VirtualMachine {
              frames: Vec::new(),
              builtins: Rc::clone(&builtins),
              modules,
              globals,
              #[cfg(feature = "jit")]
             jit: RefCell::new(JitCompiler::new()),
              profile: RefCell::new(HashMap::new()),
               last_error_line: None,
               last_error_file: None,
               last_traceback: Vec::new(),
               frame_pool: Vec::new(),
               type_registry: HashMap::new(),
               exc_type: None,
               exc_value: None,
               exc_traceback: None,
               recursion_limit: 1000,
           };
         vm.populate_type_registry();
         vm.install_source_defined_stdlib("collections", crate::modules::COLLECTIONS_USER_TYPES_SOURCE, &["UserList", "UserDict", "UserString", "Counter", "defaultdict", "ChainMap"]);
         // contextlib no longer native — real Lib/contextlib.py already defines ContextDecorator
         vm.install_source_defined_stdlib("functools", crate::modules::FUNCTOOLS_EXTRA_SOURCE, &["lru_cache", "cache"]);
         vm.install_source_defined_stdlib("enum", crate::modules::ENUM_SOURCE, &[
             "auto", "nonmember", "member", "property", "EnumType", "EnumMeta",
             "Enum", "IntEnum", "StrEnum", "unique",
         ]);
         vm.install_source_defined_stdlib("gettext", crate::modules::GETTEXT_SOURCE, &[
             "NullTranslations", "GNUTranslations", "find", "translation", "install",
             "textdomain", "bindtextdomain", "gettext", "ngettext", "pgettext", "npgettext",
             "dgettext", "dngettext", "__all__",
         ]);
         vm.install_source_defined_stdlib("json", crate::modules::JSON_EXTRA_SOURCE, &["JSONEncoder", "dumps"]);
         vm
    }

    /// Some stdlib classes are far easier (and more correct) to express as
    /// real Python source — the same way CPython's own stdlib does it — than
    /// as hand-written Rust closures (e.g. anything relying on composition
    /// over a `self.data` attribute, decorators, or `with`). This compiles
    /// and runs that source against its own dedicated, isolated globals
    /// dict (never `self.globals` — the VM's real, shared top-level
    /// namespace) and merges the requested names into the given
    /// already-registered native module's dict. Using a dedicated dict
    /// (rather than running against `self.globals` and stripping the
    /// requested names back out afterward, which this used to do) matters
    /// for correctness, not just tidiness: any exported function/class
    /// whose body references ANOTHER exported name (e.g. gettext's
    /// `translation()` referencing `GNUTranslations`) keeps working
    /// correctly forever, since the dict its closure captured is never
    /// mutated again — stripping names out from underneath it broke exactly
    /// this pattern.
    fn install_source_defined_stdlib(&mut self, module_name: &str, source: &str, names: &[&str]) {
        // Every `VirtualMachine::new()` — including the many *disposable*
        // ones spun up for nested Python-level calls from Rust builtin code
        // (a separate, documented architectural gap — see
        // `call_bound_method`'s doc comments) — used to re-parse, re-compile,
        // AND re-EXECUTE this same, never-changing Python source from
        // scratch. That's cheap in isolation but catastrophic under real
        // workloads: a single Django import chain observed here triggers
        // 2000+ disposable VMs, each redoing all of this, dominating a
        // 56-SECOND import of `django.db.models`. First fixed the
        // parse+compile half by caching the compiled `CodeObject` (cut it to
        // ~28s) — this caches the EXECUTION RESULT too: the extracted
        // `(name, PyObjectRef)` pairs (e.g. `collections.Counter`,
        // `enum.Enum` — real Type objects with their own dict of methods)
        // are stored once and simply Rc-cloned into every subsequent VM's
        // module dict instead of re-running ~800 lines of bytecode (class
        // bodies, method definitions, docstrings) per VM. This is safe
        // because these are conceptually process-global stdlib singletons —
        // exactly how CPython's own module system treats them (one `Counter`
        // class shared by every importer) — sharing the actual objects
        // across VM instances doesn't change any observable behavior for
        // user code (isinstance/identity/method dispatch all still work,
        // since it's genuinely the same objects everywhere).
        thread_local! {
            static COMPILED_STDLIB_CACHE: std::cell::RefCell<HashMap<String, Rc<CodeObject>>> = std::cell::RefCell::new(HashMap::new());
            static EXECUTED_STDLIB_CACHE: std::cell::RefCell<HashMap<String, Rc<Vec<(String, PyObjectRef)>>>> = std::cell::RefCell::new(HashMap::new());
        }
        if let Some(cached_extracted) = EXECUTED_STDLIB_CACHE.with(|c| c.borrow().get(module_name).cloned()) {
            if let Some(module) = self.modules.get(module_name) {
                if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                    for (name, obj) in cached_extracted.iter() {
                        dict.insert_str(name, obj.clone());
                    }
                }
            }
            return;
        }
        let cached = COMPILED_STDLIB_CACHE.with(|c| c.borrow().get(module_name).cloned());
        let code = match cached {
            Some(rc) => (*rc).clone(),
            None => {
                let mut parser = Parser::new(source);
                let program = match parser.parse_program() {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let mut compiler = Compiler::new();
                let code = match compiler.compile(&program, &format!("<{}>", module_name)) {
                    Ok(c) => c,
                    Err(_) => return,
                };
                COMPILED_STDLIB_CACHE.with(|c| c.borrow_mut().insert(module_name.to_string(), Rc::new(code.clone())));
                code
            }
        };
        // Real modules always have __name__ in their globals — class bodies
        // compiled inside this source (e.g. collections.Counter, a real
        // `class Counter(dict): ...`) now implicitly do `__module__ =
        // __name__` as their first statement (see compile_class_body), which
        // would otherwise NameError here since this dict starts empty.
        let dedicated_globals = Rc::new(RefCell::new(HashMap::from([
            (interner::intern("__name__"), py_str(module_name)),
        ])));
        if self.exec_code(code, Some(Rc::clone(&dedicated_globals))).is_err() {
            return;
        }
        let extracted: Vec<(String, PyObjectRef)> = {
            let globals = dedicated_globals.borrow();
            names.iter().filter_map(|name| globals.get(&interner::intern(name)).cloned().map(|v| (name.to_string(), v))).collect()
        };
        EXECUTED_STDLIB_CACHE.with(|c| c.borrow_mut().insert(module_name.to_string(), Rc::new(extracted.clone())));
        if let Some(module) = self.modules.get(module_name) {
            if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                for (name, obj) in extracted {
                    dict.insert_str(&name, obj);
                }
            }
        }
    }

    fn acquire_frame(
        &mut self,
        code: Rc<CodeObject>,
        globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
        builtins: Rc<HashMap<StrId, PyObjectRef>>,
        module_globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
    ) -> Frame {
        if let Some(mut frame) = self.frame_pool.pop() {
            let nlocals = code.nlocals;
            let names_len = code.names.len();
            let instr_len = code.instructions.len();
            frame.code = code;
            frame.globals = globals;
            frame.builtins = builtins;
            frame.module_globals = module_globals;
            frame.fast_locals.clear();
            frame.fast_locals.resize(nlocals, None);
            frame.locals.clear();
            frame.stack.clear();
            frame.ip = 0;
            frame.base_sp = 0;
            frame.exception_handlers.clear();
            frame.closure.clear();
            frame.active_exception = None;
            frame.attr_cache.clear();
            frame.attr_cache.resize(names_len, None);
            frame.global_cache.clear();
            frame.global_cache.resize(instr_len, None);
            frame.registers.clear();
            frame.name_order = None;
            frame.live_module = None;
            frame
        } else {
            Frame::new(code, globals, builtins, module_globals)
        }
    }

    fn release_frame(&mut self, frame: Frame) {
        if self.frame_pool.len() < 32 {
            self.frame_pool.push(frame);
        }
    }

    pub fn run(&mut self, code: CodeObject) -> PyResult<PyObjectRef> {
        // Real CPython always has a `__main__` module in `sys.modules`
        // backed by the running script's own globals — `__import__("__main__")`
        // (which `unittest.main()` calls unconditionally via
        // `TestProgram.__init__`'s `self.module = __import__(module)`) relies
        // on this. Without it every `if __name__ == "__main__": unittest.main()`
        // trailer in a real CPython test file raised `ImportError: No module
        // named '__main__'` instead of actually running the tests. Reuse the
        // existing `live_module` mirroring machinery (same mechanism a
        // regular file-backed module import already uses) so STORE_NAME/
        // DELETE_NAME at top level keep this module's `dict` in sync as the
        // script executes, instead of only registering it once, empty then finished.
        let main_module = self.modules.entry("__main__".to_string())
            .or_insert_with(|| create_module("__main__", HashMap::new()))
            .clone();
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(mod_dict) = dict.get_str("modules") {
                    // `sys.modules` is a real `dict` — `set_attribute` sets
                    // an OBJECT ATTRIBUTE (routed to `PyObject::Dict`'s own
                    // catch-all side-attribute-storage arm for non-Instance
                    // builtins), not a dict KEY, so this silently failed to
                    // make `"__main__"` actually appear via `sys.modules[...]`/
                    // `in sys.modules` at all — only `self.modules` (this
                    // VM's own Rust-side registry, which `import __main__`
                    // itself consults) ever really had it. Confirmed via the
                    // simplest repro: `import __main__` succeeds but
                    // `"__main__" in sys.modules` is `False` right after.
                    if let PyObject::Dict(d) = &mut *mod_dict.borrow_mut() {
                        let _ = d.set(py_str("__main__"), main_module.clone());
                    }
                }
            }
        }
        // JIT compilation disabled — using stable interpreter path only
        let mut frame = self.acquire_frame(
            Rc::new(code),
            self.globals.clone(),
            Rc::clone(&self.builtins),
            None,
        );
        frame.live_module = Some(main_module);
        self.frames.push(frame);
        let result = self.execute();
        if let Some(frame) = self.frames.pop() {
            self.release_frame(frame);
        }
        result
    }

    pub fn exec_code(&mut self, code: CodeObject, globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>) -> PyResult<PyObjectRef> {
        self.exec_code_with_module(code, globals, None)
    }

    /// Like `exec_code`, but when `live_module` is Some, every STORE_NAME/
    /// DELETE_NAME during this execution also mirrors into that module's
    /// own `dict` immediately — not just once execution finishes (see
    /// `Frame::live_module`'s doc comment for why this matters for
    /// circular imports).
    pub fn exec_code_with_module(&mut self, code: CodeObject, globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>, live_module: Option<PyObjectRef>) -> PyResult<PyObjectRef> {
        let g = globals.unwrap_or_else(|| self.globals.clone());
        let mut frame = self.acquire_frame(Rc::new(code), g, Rc::clone(&self.builtins), None);
        frame.live_module = live_module;
        self.frames.push(frame);
        let result = self.execute();
        if let Some(frame) = self.frames.pop() {
            self.release_frame(frame);
        }
        result
    }

    /// Populate the type registry with type objects for all builtin types.
    /// This is called during VM initialization so that builtin_type_of()
    /// can return real Type objects instead of string names.
    pub fn populate_type_registry(&mut self) {
        let type_names = [
            "NoneType", "bool", "int", "float", "str", "bytes", "bytearray",
            "list", "tuple", "dict", "set", "frozenset", "range", "slice",
            "function", "builtin_function_or_method", "builtin_method",
            "module", "type", "cell", "method", "partial", "property",
            "staticmethod", "classmethod", "generator", "coroutine",
            "Exception", "super", "lock", "RLock", "Event", "Queue",
            "Thread", "file", "socket", "capsule", "re.Pattern",
            "future_await_iterator", "enumerate", "list_iterator",
            "range_iterator",
        ];
        for name in &type_names {
            let type_obj = PyObjectRef::new(PyObject::Type {
                name: name.to_string(),
                dict: Box::new(TypeDict::default()),
                bases: vec![],
                mro: vec![],
            });
            self.type_registry.insert(name.to_string(), type_obj);
        }
    }

    /// Return the cached module for `name` if it's genuinely still imported
    /// (`sys.modules` has it). If it was `del sys.modules['x']`'d, build a
    /// FRESH module object (sharing the dict contents) and re-register it in
    /// both maps — real Python re-imports the module, and for a native module
    /// a fresh object is the faithful equivalent (test_atexit's
    /// test_atexit_instances asserts `atexit2 is not atexit1` while both
    /// share the same callback registry).
    pub fn import_cached_or_fresh(&mut self, name: &str) -> Option<PyObjectRef> {
        let module = self.modules.get(name)?.clone();
        let in_sys_modules = if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(mod_dict) = dict.get_str("modules") {
                    let md = mod_dict.borrow();
                    if let PyObject::Dict(d) = &*md {
                        d.get(&crate::object::py_str(name)).ok().flatten().is_some()
                    } else { false }
                } else { false }
            } else { false }
        } else { false };
        if in_sys_modules {
            return Some(module);
        }
        let fresh = PyObjectRef::new(PyObject::Module {
            name: name.to_string(),
            dict: {
                let b = module.borrow();
                if let PyObject::Module { dict, .. } = &*b {
                    dict.clone()
                } else { Box::new(crate::object::TypeDict::default()) }
            },
        });
        self.modules.insert(name.to_string(), fresh.clone());
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(mod_dict) = dict.get_str("modules") {
                    if let PyObject::Dict(d) = &mut *mod_dict.borrow_mut() {
                        let _ = d.set(crate::object::py_str(name), fresh.clone());
                    }
                }
            }
        }
        Some(fresh)
    }

    pub fn import_module_from_file(&mut self, name: &str) -> PyResult<PyObjectRef> {        // Guard against genuine infinite import recursion with a clean
        // error (showing the exact chain) instead of a raw stack overflow —
        // kept permanently (env-gated print is always-on; the depth check
        // itself is cheap) rather than added back by hand each time.
        thread_local! {
            static IMPORT_CHAIN: RefCell<Vec<String>> = RefCell::new(Vec::new());
        }
        let depth = IMPORT_CHAIN.with(|c| c.borrow().len());
        if depth > 150 {
            let chain = IMPORT_CHAIN.with(|c| c.borrow().join(" -> "));
            return Err(PyError::ImportError(format!("import recursion too deep, likely a genuine cycle: {} -> {}", chain, name)));
        }
        IMPORT_CHAIN.with(|c| c.borrow_mut().push(name.to_string()));
        if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
            eprintln!("{}IMPORT_FILE: {} (self.modules.len()={}, sys.path={:?})", "  ".repeat(depth), name, self.modules.len(),
                self.modules.get("sys").and_then(|m| if let PyObject::Module { dict, .. } = &*m.borrow() {
                    dict.get_str("path").map(|p| p.str())
                } else { None }));
        }
        let result = self.import_module_from_file_inner(name);
        IMPORT_CHAIN.with(|c| { c.borrow_mut().pop(); });
        result
    }

    fn import_module_from_file_inner(&mut self, name: &str) -> PyResult<PyObjectRef> {
        if cfg!(feature = "profile") {
            if let Ok(status) = std::fs::read_to_string(format!("/proc/{}/status", std::process::id())) {
                if let Some(_rss_line) = status.lines().find(|l| l.starts_with("VmRSS:")) {
                }
                if let Some(_peak_line) = status.lines().find(|l| l.starts_with("VmPeak:")) {
                }
            }
        }
        // Handle dotted names: e.g. "certifi.core" or "django.utils.version"
        // Walk through each segment, importing missing packages as we go
        if let Some(_dot_pos) = name.find('.') {
            let parts: Vec<&str> = name.split('.').collect();
            let mut current_name = parts[0].to_string();
            let mut parent_path: Option<String> = None;

            // A multi-part dotted import (e.g. `import django.template.engine`)
            // must initialize each ancestor package in order first, matching
            // real Python's import semantics. Without this, when `django`
            // isn't already cached, the code below falls through to a
            // direct full-path file lookup ("django/template/engine.py")
            // that finds the leaf file directly, silently skipping every
            // intermediate package's __init__.py — including module-level
            // side effects (signal registration, singletons like
            // `engines = EngineHandler()`) that code loaded later
            // transitively depends on already having run.
            if !self.modules.contains_key(&current_name) {
                let _ = self.import_module_from_file(&current_name);
            }
            // Check if we already have the top-level module
            if !self.modules.contains_key(&current_name) {
                if cfg!(feature = "profile") { eprintln!("DEBUG import: top-level '{}' NOT in modules", current_name); }
                // Not in modules — fall through to regular file search below
            } else {
                // Walk the chain: for each part after the first, resolve the child
                let mut all_resolved = true;
                for i in 1..parts.len() {
                    let child = parts[i];
                    let full_name = format!("{}.{}", current_name, child);

                    // If already in modules, skip to next
                    if self.modules.contains_key(&full_name) {
                        current_name = full_name;
                        parent_path = None;
                        continue;
                    }

                    // Get the parent's __path__
                    if parent_path.is_none() {
                        if let Some(parent_mod) = self.modules.get(&current_name) {
                            let borrowed = parent_mod.borrow();
                            if let PyObject::Module { dict, .. } = &*borrowed {
                                let p = dict.get_str("__path__").and_then(|pl| {
                                    if let PyObject::List(items) = &*pl.borrow() {
                                        items.first().and_then(|i| {
                                            if let PyObject::Str(s) = &*i.borrow() { Some(s.to_string()) } else { None }
                                        })
                                    } else { None }
                                });
                                parent_path = p;
                            } else {
                                parent_path = None;
                            }
                        } else {
                            parent_path = None;
                        }
                    }

                    // Try to find the child as a file/subpackage in parent's __path__
                    if let Some(ref base) = parent_path {
                        let base_trimmed = base.trim_end_matches('/');
                        let mut found_child = false;
                        for candidate in &[
                            format!("{}/{}.py", base_trimmed, child),
                            format!("{}/{}/__init__.py", base_trimmed, child),
                        ] {
                            if let Some(source) = self.read_module_source(candidate)? {
                                found_child = true;
                                let is_pkg = candidate.ends_with("__init__.py");
                                let empty_dict = if is_pkg {
                                    if let Some(pkg_dir) = std::path::Path::new(candidate).parent() {
                                        HashMap::from([
                                            ("__path__".to_string(), py_list(vec![py_str(&pkg_dir.to_string_lossy().to_string())])),
                                            ("__package__".to_string(), py_str(&full_name)),
                                        ])
                                    } else { HashMap::new() }
                                } else { HashMap::new() };
                                let empty_mod = create_module(&full_name, empty_dict);
                                self.modules.insert(full_name.clone(), empty_mod.clone());
                                // Register in sys.modules BEFORE executing (needed by code that checks sys.modules[__name__])
                                // Using cloned PyObjectRef to avoid holding borrow across exec_module_source
                                let sys_modules = self.modules.get("sys").and_then(|m| {
                                    let b = m.borrow();
                                    match &*b {
                                        PyObject::Module { dict, .. } => dict.get_str("modules").cloned(),
                                        _ => None,
                                    }
                                });
                                if let Some(sm) = sys_modules {
                                    // Use try_borrow_mut to avoid RefCell panic if already borrowed
                                    match &sm {
                                        PyObjectRef::Mut(rc) => {
                                            if let Ok(mut guard) = rc.try_borrow_mut() {
                                                if let PyObject::Dict(ref mut d) = &mut *guard {
                                                    d.set(py_str(&full_name), empty_mod.clone()).ok();
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                // Execute the module source
                                let module = self.exec_module_source(&source, candidate, &full_name)?;
                                self.modules.insert(full_name.clone(), module.clone());
                                // Wire into parent module namespace
                                if let Some(dot_pos) = full_name.rfind('.') {
                                    let parent_name = full_name[..dot_pos].to_string();
                                    let child_name = full_name[dot_pos+1..].to_string();
                                    if let Some(parent_mod) = self.modules.get(&parent_name).cloned() {
                                        if let PyObject::Module { dict, .. } = &mut *parent_mod.borrow_mut() {
                                            dict.insert_str(&child_name, module.clone());
                                        }
                                    }
                                }
                                current_name = full_name;
                                parent_path = None;
                                break;
                            }
                        }
                        if !found_child {
                            // Neither `child.py` nor `child/__init__.py` exists
                            // under the parent's __path__ — this dotted
                            // component isn't a submodule (could be a plain
                            // attribute of the parent, e.g. `from pkg import
                            // some_function`, or genuinely missing). Previously
                            // falling through here silently left `current_name`
                            // pointing at the last-resolved ANCESTOR package,
                            // and `all_resolved` stayed true, so the caller
                            // returned that ancestor module mislabeled as the
                            // full dotted name — e.g. `import pkg.missing`
                            // would silently succeed, yielding `pkg` itself.
                            all_resolved = false;
                            break;
                        }
                    } else {
                        all_resolved = false;
                        break;
                    }
                }
                if all_resolved {
                    if let Some(result) = self.modules.get(&current_name).cloned() {
                        return Ok(result);
                    }
                }
                // If we resolved some but not all, continue to search
                // from the last unresolved parent
            }

            // If we didn't have the top-level or couldn't walk the chain,
            // fall through to regular sys.path search below
        }

        // Search sys.path for the module
        let search_paths = self.get_sys_path();
        let py_name = name.replace('.', "/");
        for base in &search_paths {
            let py_path = if base.ends_with('/') {
                format!("{}{}.py", base, py_name)
            } else {
                format!("{}/{}.py", base, py_name)
            };
            if let Some(source) = self.read_module_source(&py_path)? {
                let empty_mod = create_module(name, HashMap::new());
                self.modules.insert(name.to_string(), empty_mod.clone());
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules").cloned() {
                            mod_dict.borrow_mut().set_attribute(name, empty_mod.clone()).ok();
                        }
                    }
                }
                let module = self.exec_module_source(&source, &py_path, name)?;
                self.modules.insert(name.to_string(), module.clone());
                // Wire submodule into parent module namespace and update sys.modules
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules").cloned() {
                            mod_dict.borrow_mut().set_attribute(name, module.clone()).ok();
                        }
                    }
                }
                // Wire submodule into parent module namespace
                if let Some(dot_pos) = name.rfind('.') {
                    let parent_name = name[..dot_pos].to_string();
                    let child_name = name[dot_pos+1..].to_string();
                    if let Some(parent_mod) = self.modules.get(&parent_name).cloned() {
                        if let PyObject::Module { dict, .. } = &mut *parent_mod.borrow_mut() {
                            dict.insert_str(&child_name, module.clone());
                        }
                    }
                }
                return Ok(module);
            }
            let init_path = if base.ends_with('/') {
                format!("{}{}/__init__.py", base, py_name)
            } else {
                format!("{}/{}/__init__.py", base, py_name)
            };
            if let Some(source) = self.read_module_source(&init_path)? {
                let pkg_dir = std::path::Path::new(&init_path).parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let mut empty_dict = HashMap::new();
                empty_dict.insert_str("__path__", py_list(vec![py_str(&pkg_dir)]));
                empty_dict.insert_str("__package__", py_str(name));
                let empty_mod = create_module(name, empty_dict);
                self.modules.insert(name.to_string(), empty_mod.clone());
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules").cloned() {
                            mod_dict.borrow_mut().set_attribute(name, empty_mod.clone()).ok();
                        }
                    }
                }
                let module = self.exec_module_source(&source, &init_path, name)?;
                self.modules.insert(name.to_string(), module.clone());
                // Update sys.modules with the loaded module (overwrites empty stub)
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules").cloned() {
                            mod_dict.borrow_mut().set_attribute(name, module.clone()).ok();
                        }
                    }
                }
                return Ok(module);
            }
            // Try loading as a .so C extension (requires the "ffi" feature)
            #[cfg(feature = "ffi")]
            {
                let so_path = if base.ends_with('/') {
                    format!("{}{}.cpython-313-x86_64-linux-gnu.so", base, name)
                } else {
                    format!("{}/{}.cpython-313-x86_64-linux-gnu.so", base, name)
                };
                if std::path::Path::new(&so_path).exists() {
                    // SAFETY: loading and running a CPython C extension's
                    // PyInit_* entry point is inherently unsafe — there is no
                    // way to verify the .so at `so_path` actually implements
                    // the CPython C-API contract it claims to. This is the
                    // deliberate, documented risk of the "ffi" feature: it
                    // only runs when the caller opts in by enabling it and
                    // pointing sys.path at a real compiled extension.
                    let loaded = unsafe { crate::ffi_bridge::load_extension(&so_path, name) };
                    match loaded {
                        Ok(()) => {
                            // Try to get the module from the extension registry
                            // SAFETY: see above — same trust boundary, reading
                            // state populated by the load_extension call just above.
                            if let Some(mod_obj) = unsafe { crate::ffi_bridge::get_extension_module(name) } {
                                return Ok(mod_obj);
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        Err(PyError::module_not_found_error(format!("No module named '{}'", name)))
    }

    fn get_sys_path(&self) -> Vec<String> {
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(path_list) = dict.get_str("path") {
                    if let PyObject::List(items) = &*path_list.borrow() {
                        return items.iter().filter_map(|item| {
                            if let PyObject::Str(s) = &*item.borrow() { Some(s.to_string()) } else { None }
                        }).collect();
                    }
                }
            }
        }
        vec![]
    }

    /// Read a `.py` source file off disk and decode it as real Python would:
    /// PEP 263 coding cookie if present, strict UTF-8 otherwise. Returns
    /// `Ok(None)` when the file simply doesn't exist (caller continues its
    /// path search), but PROPAGATES a decode failure — a file that exists
    /// yet isn't valid in its implied encoding is a `SyntaxError`, exactly
    /// as a real import would raise, NOT a silent "module not found".
    /// Deliberately NOT `std::fs::read_to_string` — that lossy-substitutes
    /// non-UTF-8 bytes (confirmed via `test_utf8source.py::test_badsyntax`).
    fn read_module_source(&self, path: &str) -> PyResult<Option<String>> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        crate::object::import_builtin::decode_source_bytes(&bytes).map(Some)
    }

    fn exec_module_source(&mut self, source: &str, path: &str, name: &str) -> PyResult<PyObjectRef> {
        // ── .pyc cache support ─────────────────────────────────────────
        // Try to load a previously-compiled .pyc file. If valid (matching
        // magic + version + source timestamp), skip parsing and compilation.
        const PYC_MAGIC: u32 = 0x52535079; // "RSPy"
        const PYC_VERSION: u16 = 2; // bumped: CodeObject gained kwonly_defaults_mask

        let py_path = std::path::Path::new(path);
        let source_mtime = std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut cached_code: Option<CodeObject> = None;

        // Compute __pycache__/basename.pyc path
        if let Some(parent) = py_path.parent() {
            if let Some(stem) = py_path.file_stem().and_then(|s| s.to_str()) {
                let pyc_dir = parent.join("__pycache__");
                let pyc_filename = format!("{}.rustpython-0.pyc", stem);
                let pyc_path = pyc_dir.join(&pyc_filename);

                if let Ok(pyc_data) = std::fs::read(&pyc_path) {
                    // Minimum size: magic(4) + version(2) + timestamp(8) = 14 bytes
                    if pyc_data.len() >= 14 {
                        let magic = u32::from_le_bytes([
                            pyc_data[0], pyc_data[1], pyc_data[2], pyc_data[3],
                        ]);
                        let version = u16::from_le_bytes([pyc_data[4], pyc_data[5]]);
                        let ts = u64::from_le_bytes([
                            pyc_data[6], pyc_data[7], pyc_data[8], pyc_data[9],
                            pyc_data[10], pyc_data[11], pyc_data[12], pyc_data[13],
                        ]);
                        if magic == PYC_MAGIC && version == PYC_VERSION && ts == source_mtime {
                            if let Ok(code) = CodeObject::from_bytes(&pyc_data[14..]) {
                                cached_code = Some(code);
                            }
                        }
                    }
                }
            }
        }

        // Parse and compile, or deserialise from cache
        let code: CodeObject = match cached_code {
            Some(cached) => cached,
            None => {
                let mut parser = crate::parser::Parser::new(source);
                let program = parser.parse_program()
                    .map_err(|e| PyError::RuntimeError(format!("Parse error in '{}': {}", name, e)))?;
                drop(parser);  // Free parser memory (AST is now in `program`)

                let mut compiler = crate::compiler::Compiler::new();
                let compiled = compiler.compile(&program, path)
                    .map_err(|e| PyError::RuntimeError(format!("Compile error: {}", e)))?;
                drop(compiler);  // Free compiler internal tables
                drop(program);   // Free AST — CodeObject is now self-contained

                // Write .pyc cache for future imports (skip for stdlib modules).
                // Stdlib modules under /usr/ are stable + huge; serialising them
                // costs CPU + a temporary Vec<u8> allocation, and writing usually
                // fails silently anyway due to permissions on /usr/lib/__pycache__/.
                if !path.starts_with("/usr") {
                    if let Some(parent) = py_path.parent() {
                        if let Some(stem) = py_path.file_stem().and_then(|s| s.to_str()) {
                            let pyc_dir = parent.join("__pycache__");
                            let pyc_filename = format!("{}.rustpython-0.pyc", stem);
                            let pyc_path = pyc_dir.join(&pyc_filename);

                            let mut pyc_data = Vec::new();
                            pyc_data.extend_from_slice(&PYC_MAGIC.to_le_bytes());
                            pyc_data.extend_from_slice(&PYC_VERSION.to_le_bytes());
                            pyc_data.extend_from_slice(&source_mtime.to_le_bytes());
                            pyc_data.extend_from_slice(&compiled.to_bytes());

                            let _ = std::fs::create_dir_all(&pyc_dir);
                            let _ = std::fs::write(&pyc_path, &pyc_data);
                        }
                    }
                }

                compiled
            }
        };

        let is_package = path.ends_with("__init__.py");
        let mut globals_map: HashMap<StrId, PyObjectRef> = HashMap::from([
            (interner::intern("__name__"), py_str(name)),
            (interner::intern("__file__"), py_str(path)),
            (interner::intern("__builtins__"), create_module("builtins", self.builtins.iter().map(|(k,v)| (interner::lookup_str(*k).to_string(), v.clone())).collect())),
        ]);
        if is_package {
            if let Some(pkg_dir) = std::path::Path::new(path).parent() {
                let pkg_dir_str = pkg_dir.to_string_lossy().to_string();
                globals_map.insert(interner::intern("__path__"), py_list(vec![py_str(&pkg_dir_str)]));
                globals_map.insert(interner::intern("__package__"), py_str(name));
            }
        } else {
            // For non-package modules, __package__ should be set to the parent package name
            // (e.g., "django.apps" for "django.apps.registry") so relative imports work
            let pkg = name.rfind('.').map(|dot| &name[..dot]).unwrap_or("");
            globals_map.insert(interner::intern("__package__"), 
                if pkg.is_empty() { py_str("") } else { py_str(pkg) });
        }
        let module_globals = Rc::new(RefCell::new(globals_map));
        // Register module in sys.modules BEFORE executing (needed for sys.modules[__name__] checks)
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(sm) = dict.get_str("modules").cloned() {
                    match &sm {
                        PyObjectRef::Mut(rc) => {
                            if let Ok(mut guard) = rc.try_borrow_mut() {
                                if let PyObject::Dict(ref mut d) = &mut *guard {
                                    d.set(py_str(name), py_str(&format!("<module '{}' (loaded)>", name))).ok();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // If the caller already registered a placeholder module object under
        // this name (the normal case — see import_module_from_file_inner),
        // mirror every STORE_NAME into its dict live, as execution happens,
        // instead of only after the whole body finishes (see
        // `Frame::live_module`'s doc comment). This is what lets a
        // circular import elsewhere see names this module already defined
        // even while it's still mid-execution — matching real CPython,
        // where `module.__dict__` IS the executing frame's globals, not a
        // separate snapshot.
        let live_module = self.modules.get(name).cloned();
        if let Some(lm) = &live_module {
            if let PyObject::Module { dict, .. } = &mut *lm.borrow_mut() {
                for (k, v) in module_globals.borrow().iter() {
                    dict.insert_str(interner::lookup_str(*k), v.clone());
                }
            }
        }
        // Preserve the real exception (type + object) raised by the module
        // body as-is — this used to be flattened into a formatted String via
        // `.map_err(|e| format!("{}", e))`, which meant a module raising e.g.
        // TypeError during import surfaced to the importer as a generic
        // ImportError with the TypeError's message glued into the text
        // instead of the actual TypeError, so `except TypeError` (or
        // anything other than a blanket `except ImportError`/`except
        // Exception`) around an import could never catch it.
        if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
            eprintln!("MODULE_EXEC_START: {}", name);
        }
        self.exec_code_with_module(code, Some(Rc::clone(&module_globals)), live_module)?;
        if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
            eprintln!("MODULE_EXEC_DONE: {}", name);
        }
        let globals_copy = module_globals.borrow().clone();
        // If a placeholder module was already registered under this name
        // (e.g. by import_module_from_file, to support circular imports),
        // populate it in place rather than returning a brand new object —
        // any reference a circular importer already grabbed a clone of
        // must see the final contents too, not just IMPORT_FROM's own
        // live-frame fallback (which only covers names accessed while
        // still mid-execution).
        if let Some(existing) = self.modules.get(name).cloned() {
            if let PyObject::Module { dict, .. } = &mut *existing.borrow_mut() {
                for (k, v) in globals_copy.iter() { dict.insert_str(interner::lookup_str(*k), v.clone()); }
            }
            return Ok(existing);
        }
        Ok(create_module(name, globals_copy.into_iter().map(|(k,v)| (interner::lookup_str(k).to_string(), v)).collect()))
    }

    /// Try to execute a simple function without creating a Frame.
    /// Returns Some(result) if the function was simple enough, None otherwise.
    fn try_exec_simple(code: &CodeObject, args: &[PyObjectRef]) -> Option<PyResult<PyObjectRef>> {
        if code.vararg_name.is_some() || code.kwarg_name.is_some() || code.num_defaults > 0 {
            return None;
        }
        // A wrong argument count must raise `TypeError` (see the slow path's
        // own validation just below in `call_function`) — this fast path
        // has no such check of its own, so falling through to the slow path
        // whenever the count doesn't match exactly is what makes that
        // validation actually apply to every call, not just the ones that
        // happen to miss this "simple function" fast path.
        if args.len() != code.arg_count {
            return None;
        }
        let instrs = &code.instructions;
        if instrs.is_empty() || instrs.len() > 12 {
            return None;
        }
        // Pre-allocate local variables from arguments
        let mut locals: Vec<Option<PyObjectRef>> = vec![None; code.varnames.len()];
        for (i, arg) in args.iter().enumerate() {
            if i < locals.len() {
                locals[i] = Some(arg.clone());
            }
        }
        let mut stack: SmallVec<[PyObjectRef; 8]> = SmallVec::new();
        let mut ip: usize = 0;
        let n_instrs = instrs.len();
        loop {
            if ip >= n_instrs { return None; }
            let instr = &instrs[ip];
            ip += 1;
            match instr.op {
                Opcode::LOAD_FAST => {
                    let idx = instr.arg as usize;
                    let val = locals.get(idx)?.clone()?;
                    stack.push(val);
                }
                Opcode::STORE_FAST => {
                    let idx = instr.arg as usize;
                    let val = stack.pop()?;
                    if idx < locals.len() { locals[idx] = Some(val); }
                }
                Opcode::LOAD_CONST => {
                    // Shares `eval_const_value`/`const_cache` with the main
                    // `execute_instruction` LOAD_CONST handler — this fast
                    // path (no-`Frame` "simple function" execution) used to
                    // have its OWN independent copy of the same parsing
                    // logic, uncached, and using `s.trim_start_matches('_')`
                    // (only strips LEADING underscores) instead of the main
                    // copy's `s.chars().filter(|&c| c != '_')` (strips ALL
                    // of them) — an inconsistency that would have mis-parsed
                    // a mid-literal digit separator like `1_000_000`
                    // differently depending on which path happened to
                    // execute a given function.
                    let const_idx = instr.arg as usize;
                    let cached = code.const_cache.borrow().get(const_idx).and_then(|c| c.clone());
                    let obj = if let Some(obj) = cached {
                        obj
                    } else {
                        let const_val = code.consts.get(const_idx)?.clone();
                        // Only Function/Complex/Bytes/Tuple/Code consts are
                        // NOT handled here (this fast path only ever runs
                        // for "simple" functions — see this fn's own doc
                        // comment — which realistically only ever load
                        // None/Bool/Int/Float/String constants); fall back
                        // to the slow path for anything else via `None`.
                        if !matches!(const_val, ConstValue::None | ConstValue::Bool(_) | ConstValue::Int(_) | ConstValue::Float(_) | ConstValue::String(_)) {
                            return None;
                        }
                        let obj = eval_const_value(const_val).ok()?;
                        let mut cache = code.const_cache.borrow_mut();
                        if cache.len() <= const_idx {
                            cache.resize(const_idx + 1, None);
                        }
                        cache[const_idx] = Some(obj.clone());
                        obj
                    };
                    stack.push(obj);
                }
                Opcode::BINARY_OP => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let result = match instr.arg {
                        0 => py_add(&left, &right),
                        1 => py_sub(&left, &right),
                        2 => py_mul(&left, &right),
                        3 => py_div(&left, &right),
                        4 => py_floor_div(&left, &right),
                        5 => py_mod(&left, &right),
                        6 => py_pow(&left, &right),
                        7 => py_lshift(&left, &right),
                        8 => py_rshift(&left, &right),
                        9 => py_bit_or(&left, &right),
                        10 => py_bit_xor(&left, &right),
                        11 => py_bit_and(&left, &right),
                        13 => py_getitem(&left, &right),
                        _ => return None,
                    };
                    match result { Ok(v) => stack.push(v), Err(e) => return Some(Err(e)) }
                }
                Opcode::COMPARE_OP => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let result = py_compare(&left, &right, instr.arg);
                    match result { Ok(v) => stack.push(v), Err(e) => return Some(Err(e)) }
                }
                Opcode::POP_JUMP_IF_FALSE => {
                    let val = stack.pop()?;
                    if !val.truthy() { ip = instr.arg as usize; }
                }
                Opcode::JUMP_FORWARD => {
                    ip = ip + instr.arg as usize;
                }
                Opcode::JUMP_BACKWARD => {
                    ip = ip - (instr.arg as usize + 1);
                }
                Opcode::RETURN_VALUE => return Some(Ok(stack.pop()?)),
                Opcode::LOAD_ATTR => {
                    let obj = stack.pop()?;
                    let name_id = code.names[instr.arg as usize];
                    let name = crate::interner::lookup_str(name_id);
                    let val = obj.borrow().get_attribute(name);
                    match val { Ok(v) => stack.push(v), Err(e) => return Some(Err(e)) }
                }
                _ => return None,
            }
        }
    }

    pub fn execute(&mut self) -> PyResult<PyObjectRef> {
        crate::object::VM_PTR.with(|p| {
            let needs_set = p.borrow().is_none();
            if needs_set {
                *p.borrow_mut() = Some(self as *mut VirtualMachine);
            }
        });
        // Every call site that pushes a frame onto `self.frames` immediately
        // calls `execute()` and pops exactly that one frame once it returns
        // (see exec_code, call_function's Function arm, __build_class__,
        // generator/coroutine drivers) — so for the entire lifetime of this
        // `execute_inner` invocation, `self.frames[frame_floor]` is *this*
        // call's own frame, and any frames below it belong to an outer,
        // currently-suspended `execute()` call further down the Rust stack.
        // Bounding exception handling to `frame_floor` matters: without it,
        // an uncaught exception from a nested call (a Python function call,
        // a module body during import, ...) would find and "handle" itself
        // using an outer/caller frame's try/except — while that outer frame
        // was not actually the one executing, and the intervening frame(s)
        // were never popped/unwound. Instead, nested calls must propagate an
        // unhandled exception as a plain Err all the way back to their own
        // call site (which pops its own frame), letting the *caller's own*
        // execute_inner loop (now correctly with its own frame on top) find
        // the enclosing handler itself.
        let frame_floor = self.frames.len() - 1;
        let result = self.execute_inner(frame_floor);
        // Store exception info for sys.exc_info()
        if let Err(ref e) = result {
            // Must be the real exception object + its real class (what
            // `sys.exc_info()` returns), not a bare descriptive string —
            // this is what let `issubclass(sys.exc_info()[0], ...)` crash
            // with "arg 1 must be a class" for ANY natively-raised error
            // (a `TypeError`/`ValueError`/etc. raised internally by a
            // builtin/opcode rather than a Python-level `raise` statement,
            // which instead goes through RAISE_VARARGS's own now-fixed
            // assignment) — exactly the same bug, just a second, separate
            // site that produced it for a different class of raise.
            let exc_obj = Self::error_to_exc_obj(e);
            self.exc_type = Some(self.exception_class_of(&exc_obj));
            self.exc_value = Some(exc_obj);
        }
        result
    }

    /// Injects `err` at the current suspension point of the single frame
    /// already pushed onto `self.frames` (used by generator/coroutine
    /// `.throw()`), then resumes normal execution. Mirrors `execute()`'s
    /// frame_floor bookkeeping but starts by searching for a handler for
    /// `err` instead of running the next instruction — this is what lets a
    /// `try/finally` wrapping the suspended `yield` actually see the thrown
    /// exception and run its cleanup, exactly as CPython's generator throw
    /// does. Returns Err(err) unchanged if the generator's own code has no
    /// handler for it (caller propagates it to whoever called .throw()).
    pub(crate) fn throw_into_frame(&mut self, err: PyError) -> PyResult<PyObjectRef> {
        let frame_floor = self.frames.len() - 1;
        if !self.handle_exception(&err, frame_floor) {
            return Err(err);
        }
        let result = self.execute_inner(frame_floor);
        if let Err(ref e) = result {
            // Must be the real exception object + its real class (what
            // `sys.exc_info()` returns), not a bare descriptive string —
            // this is what let `issubclass(sys.exc_info()[0], ...)` crash
            // with "arg 1 must be a class" for ANY natively-raised error
            // (a `TypeError`/`ValueError`/etc. raised internally by a
            // builtin/opcode rather than a Python-level `raise` statement,
            // which instead goes through RAISE_VARARGS's own now-fixed
            // assignment) — exactly the same bug, just a second, separate
            // site that produced it for a different class of raise.
            let exc_obj = Self::error_to_exc_obj(e);
            self.exc_type = Some(self.exception_class_of(&exc_obj));
            self.exc_value = Some(exc_obj);
        }
        result
    }

    fn execute_inner(&mut self, frame_floor: usize) -> PyResult<PyObjectRef> {
        loop {
            let result = self.execute_instruction();
            match result {
                Ok(None) => continue,
                Ok(Some(val)) => return Ok(val),
                Err(e) => {
                    if matches!(&e, PyError::SystemExit(_)) {
                        return Err(e);
                    }
                    if std::env::var("RPY_DEBUG_EXC").is_ok() {
                        eprintln!("handle_exception: frame_floor={} frames.len()={} err={}", frame_floor, self.frames.len(), e);
                    }
                    if !self.handle_exception(&e, frame_floor) {
                        // This execute() call's own frame has no handler for `e` — it
                        // will propagate as a plain Err up to our Rust caller, which
                        // pops this frame. Record this frame's info before that
                        // happens; as the error keeps propagating outward, each
                        // enclosing execute() level prepends its own frame here too,
                        // building the traceback outermost-first (only cleared when
                        // some level below DOES catch it — see the `else` branch).
                        if let Some(f) = self.frames.get(frame_floor) {
                            let idx = f.ip.saturating_sub(1).min(f.code.instructions.len().saturating_sub(1));
                            let line = f.code.line_number(idx);
                            // Each enclosing level re-runs this same branch as
                            // the error keeps propagating outward — only the
                            // FIRST (innermost, deepest) occurrence should set
                            // `last_error_line`/`file` (matching the ORIGINAL
                            // per-instruction-update behavior, which always
                            // reflected wherever execution last was, i.e. the
                            // innermost frame, before the error started
                            // unwinding). `last_traceback` is still empty only
                            // on this first, innermost pass.
                            if self.last_traceback.is_empty() {
                                self.last_error_line = Some(line);
                                self.last_error_file = Some(crate::interner::lookup_str(f.code.filename).to_string());
                            }
                            self.last_traceback.insert(0, (crate::interner::lookup_str(f.code.filename).to_string(), line, crate::interner::lookup_str(f.code.name).to_string()));
                        }
                        return Err(e);
                    } else {
                        // Exception was actually caught somewhere — any traceback
                        // entries accumulated so far (from inner frames that didn't
                        // handle it) no longer describe a real escaping error.
                        self.last_traceback.clear();
                    }
                    if std::env::var("RPY_DEBUG_EXC").is_ok() {
                        eprintln!("  handled: frames.len()={} top_stack_len={}", self.frames.len(), self.frames.last().map(|f| f.stack.len()).unwrap_or(0));
                    }
                }
            }
        }
    }

    fn execute_instruction(&mut self) -> PyResult<Option<PyObjectRef>> {
        let fi = self.frames.len() - 1;
        let ip = self.frames[fi].ip;
        if ip >= self.frames[fi].code.instructions.len() {
            return Err(PyError::runtime_error("execution reached end of code"));
        }
        // `last_error_line`/`last_error_file` (only ever read once, in
        // `main.rs`'s final top-level uncaught-error report) used to be
        // updated HERE — unconditionally, on every single instruction
        // executed, including a `.clone()` of the filename string — instead
        // of only at the point an error actually escapes uncaught (set in
        // `execute_inner`'s error-handling branch below, right where the
        // equivalent `last_traceback` entry is already computed from the
        // exact same frame/line). For a hot loop or recursive function
        // executing millions of instructions, that was millions of
        // pointless heap allocations doing bookkeeping nothing ever reads
        // unless the program is about to crash.
        let op = self.frames[fi].code.instructions[ip].op;
        let arg = self.frames[fi].code.instructions[ip].arg;
        self.frames[fi].ip = ip + 1;
        // Debug: print instruction (only with profile feature)
        if cfg!(feature = "profile") {
            if matches!(op, Opcode::LOAD_GLOBAL | Opcode::LOAD_FAST | Opcode::CALL | Opcode::LOAD_ATTR | Opcode::RETURN_VALUE) {
                let _frame_name = &self.frames[fi].code.name;
            }
        }

        // Profile: increment counter for this instruction
        // Only in profile mode (disabled by default for speed)
        if cfg!(feature = "profile") {
            let func_id = fi; // use frame index as function identifier
            let mut prof = self.profile.borrow_mut();
            let counters = prof.entry(func_id).or_insert_with(|| vec![0u32; self.frames[fi].code.instructions.len()]);
            if ip < counters.len() {
                counters[ip] = counters[ip].saturating_add(1);
            }
        }

        match op {
            Opcode::NOP => {}

            Opcode::LOAD_CONST => {
                let const_idx = arg as usize;
                // Fast path: this exact LOAD_CONST was already parsed once
                // before (by this same CodeObject, possibly on a PRIOR call
                // — the cache lives on the `Rc`-shared CodeObject itself,
                // not the per-call Frame) — every `consts[i]` value is a
                // pure, deterministic source of one Python value, so a
                // cached hit is always correct, never stale (see
                // `CodeObject::const_cache`'s own doc comment for why this
                // is safe unlike the Frame-level LOAD_ATTR/LOAD_GLOBAL
                // caches, which read genuinely mutable state).
                let cached = self.frames[fi].code.const_cache.borrow()
                    .get(const_idx).and_then(|c| c.clone());
                let obj = if let Some(obj) = cached {
                    obj
                } else {
                    let const_val = self.frames[fi].code.consts.get(const_idx).ok_or_else(|| {
                        PyError::runtime_error(format!("constant index out of range: {}", const_idx))
                    })?.clone();
                    let obj = eval_const_value(const_val)?;
                    let mut cache = self.frames[fi].code.const_cache.borrow_mut();
                    if cache.len() <= const_idx {
                        cache.resize(const_idx + 1, None);
                    }
                    cache[const_idx] = Some(obj.clone());
                    obj
                };
                self.frames[fi].push(obj);
            }

            Opcode::LOAD_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.get_local(name).cloned()
                        .or_else(|| {
                            // A class body's own namespace (checked via get_local
                            // above) takes priority, but if the name isn't defined
                            // there, it may still be a free variable closed over
                            // from an enclosing function — matching CPython's
                            // LOAD_CLASSDEREF fallback (class bodies skip enclosing
                            // *function* scopes when resolving names normally, but
                            // methods defined inside still need to close over them,
                            // so this frame's own code object carries them as
                            // freevars/a closure exactly like a nested function).
                            let fv_idx = f.code.freevars.iter().position(|n| n == name)?;
                            let cell = f.closure.get(fv_idx)?;
                            match &*cell.borrow() {
                                PyObject::Cell { value: Some(inner) } => Some(inner.clone()),
                                PyObject::Cell { value: None } => None,
                                _ => Some(cell.clone()),
                            }
                        })
                        .or_else(|| f.globals.borrow().get(&interner::intern(name)).cloned())
                        .or_else(|| {
                            // Check module_globals (enclosing module scope for class bodies)
                            f.module_globals.as_ref()
                                .and_then(|mg| mg.borrow().get(&interner::intern(name)).cloned())
                        })
                        .or_else(|| f.builtins.get(&interner::intern(name)).cloned())
                };
                match val {
                    Some(v) => self.frames[fi].push(v),
                    None => return Err(PyError::name_error(format!("name '{}' is not defined", name))),
                }
            }

            Opcode::STORE_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                if let Some(order) = self.frames[fi].name_order.clone() {
                    let mut order = order.borrow_mut();
                    if !order.contains(&name) {
                        order.push(name.clone());
                    }
                }
                if let Some(live_module) = self.frames[fi].live_module.clone() {
                    if let PyObject::Module { dict, .. } = &mut *live_module.borrow_mut() {
                        dict.insert_str(&name, val.clone());
                    }
                }
                self.frames[fi].globals.borrow_mut().insert(interner::intern(&name), val);
            }

            Opcode::LOAD_FAST => {
                let var_idx = arg as usize;
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.fast_locals.get(var_idx).and_then(|v| v.clone())
                };
                match val {
                    // Mirror LOAD_DEREF's own unwrapping: if this slot has
                    // been converted into a cell (MAKE_CELL ran for it, or
                    // a same-slot STORE_FAST landed on an existing cell —
                    // see STORE_FAST's own comment on why that can happen),
                    // push the cell's inner value, not the cell object
                    // itself. Falls through to pushing the raw value
                    // unchanged for the overwhelmingly common non-cell case.
                    Some(v) if matches!(&*v.borrow(), PyObject::Cell { .. }) => {
                        let inner = match &*v.borrow() {
                            PyObject::Cell { value: Some(inner) } => Some(inner.clone()),
                            PyObject::Cell { value: None } => None,
                            _ => unreachable!(),
                        };
                        match inner {
                            Some(inner) => self.frames[fi].push(inner),
                            None => return Err(PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                                self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s))))),
                        }
                    }
                    Some(v) => self.frames[fi].push(v),
                    None => {
                        if std::env::var("RPY_DEBUG_NAMEERROR").is_ok() {
                            eprintln!("LOAD_FAST unbound: func={} file={} line={:?} varnames={:?}",
                                self.frames[fi].code.name, self.frames[fi].code.filename,
                                self.frames[fi].code.line_number(self.frames[fi].ip.saturating_sub(1)),
                                self.frames[fi].code.varnames);
                        }
                        return Err(PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                            self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s)))));
                    }
                }
            }

            Opcode::STORE_FAST => {
                let var_idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let frame = &mut self.frames[fi];
                if var_idx < frame.fast_locals.len() {
                    // If this slot has already been converted into a cell
                    // (MAKE_CELL ran for it, e.g. because a nested
                    // scope/lambda captures the same name — real trigger:
                    // CPython's own `test_listcomps.py`, a comprehension's
                    // iteration variable ending up cellvar-classified),
                    // update the cell's VALUE in place rather than
                    // clobbering the slot with the raw value directly.
                    // Overwriting it outright previously corrupted the
                    // slot's cell-ness, so a LATER `STORE_DEREF`/`LOAD_DEREF`
                    // for the same variable would try to `.borrow_mut()` a
                    // plain (non-Mut) value instead of the expected Cell —
                    // an unconditional hard panic ("borrow_mut called on
                    // non-Mut value"), not just a semantic mismatch.
                    let is_existing_cell = matches!(&frame.fast_locals[var_idx], Some(existing) if matches!(&*existing.borrow(), PyObject::Cell { .. }));
                    if is_existing_cell {
                        if let Some(existing) = frame.fast_locals[var_idx].clone() {
                            if let PyObject::Cell { value } = &mut *existing.borrow_mut() {
                                *value = Some(val.clone());
                            }
                        }
                    } else {
                        frame.fast_locals[var_idx] = Some(val.clone());
                    }
                }
                let name = crate::interner::lookup_str(frame.code.varnames[var_idx]);
                frame.insert_local(name, val);
            }

            Opcode::LOAD_GLOBAL => {
                let instr_ip = self.frames[fi].ip - 1;  // already incremented
                // Check inline cache first
                if let Some(cached) = self.frames[fi].global_cache.get(instr_ip).and_then(|c| c.clone()) {
                    self.frames[fi].push(cached);
                } else {
                    let name_idx = arg as usize;
                    let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                    let val = {
                        let f = &self.frames[self.frames.len() - 1];
                        let v = f.globals.borrow().get(&interner::intern(name)).cloned()
                            .or_else(|| f.module_globals.as_ref()
                                .and_then(|mg| mg.borrow().get(&interner::intern(name)).cloned()))
                            .or_else(|| f.builtins.get(&interner::intern(name)).cloned());
                        v
                    };
                    match val {
                        Some(v) => {
                            // Cache for next time
                            if instr_ip < self.frames[fi].global_cache.len() {
                                self.frames[fi].global_cache[instr_ip] = Some(v.clone());
                            }
                            self.frames[fi].push(v);
                        }
                        None => return Err(PyError::name_error(format!("name '{}' is not defined", name))),
                    }
                }
            }

            Opcode::STORE_GLOBAL => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                self.frames[fi].globals.borrow_mut().insert(interner::intern(&name), val);
            }

            Opcode::LOAD_DEREF => {
                let idx = arg as usize;
                let (cell_ref, is_freevar, name_str): (Option<PyObjectRef>, bool, String) = {
                    let f = &self.frames[fi];
                    let code = &f.code;
                    if idx < code.cellvars.len() {
                        let name = &code.cellvars[idx];
                        let var_idx = code.varnames.iter().position(|&n| crate::interner::intern_eq(n, name))
                            .ok_or_else(|| PyError::name_error(format!("variable '{}' not found", name)))?;
                        (f.fast_locals[var_idx].clone(), false, name.clone())
                    } else {
                        let fv_idx = idx - code.cellvars.len();
                        let name = code.freevars.get(fv_idx)
                            .ok_or_else(|| PyError::runtime_error("freevar index out of range"))?;
                        (f.closure.get(fv_idx).cloned(), true, name.clone())
                    }
                };
                if let Some(cell) = cell_ref {
                    let val = {
                        let obj = cell.borrow();
                        match &*obj {
                            PyObject::Cell { value: Some(inner) } => inner.clone(),
                            PyObject::Cell { value: None } => {
                                return Err(PyError::name_error(format!("variable '{}' referenced before assignment", name_str)));
                            }
                            _ => cell.clone(),
                        }
                    };
                    self.frames[fi].push(val);
                } else if is_freevar {
                    let val = {
                        let globals = self.frames[fi].globals.borrow();
                        globals.get(&interner::intern(&name_str)).cloned()
                    };
                    if let Some(v) = val {
                        self.frames[fi].push(v);
                    } else {
                        let val = self.frames[fi].builtins.get(&interner::intern(&name_str)).cloned();
                        if let Some(v) = val {
                            self.frames[fi].push(v);
                        } else {
                            return Err(PyError::name_error(format!("variable '{}' not found", name_str)));
                        }
                    }
                } else {
                    return Err(PyError::name_error(format!("variable '{}' not found", name_str)));
                }
            }

            Opcode::STORE_DEREF => {
                let idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let has_cellvars = idx < self.frames[fi].code.cellvars.len();
                if has_cellvars {
                    let name = &self.frames[fi].code.cellvars[idx];
                    let var_idx = self.frames[fi].code.varnames.iter().position(|&n| crate::interner::intern_eq(n, name))
                        .ok_or_else(|| PyError::runtime_error("variable not found"))?;
                    if var_idx < self.frames[fi].fast_locals.len() {
                        // Defensive: only treat the existing slot value as
                        // a real cell if it actually is one. A same-slot
                        // plain `STORE_FAST` landing here first (see that
                        // opcode's own comment) is now handled there, but
                        // this guards against any OTHER way the slot could
                        // end up non-cell — safe fallback (build a fresh
                        // cell) instead of `.borrow_mut()` unconditionally
                        // panicking on a non-`Mut` value.
                        let existing_is_cell = matches!(&self.frames[fi].fast_locals[var_idx], Some(c) if matches!(&*c.borrow(), PyObject::Cell { .. }));
                        if existing_is_cell {
                            let cell = self.frames[fi].fast_locals[var_idx].clone().unwrap();
                            let mut cell_val = cell.borrow_mut();
                            if let PyObject::Cell { value } = &mut *cell_val {
                                *value = Some(val);
                            }
                        } else {
                            let new_cell = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                            self.frames[fi].fast_locals[var_idx] = Some(new_cell);
                        }
                    } else {
                        let new_cell = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                        self.frames[fi].fast_locals.push(Some(new_cell));
                    }
                } else {
                    let fv_idx = idx - self.frames[fi].code.cellvars.len();
                    let existing_is_cell = matches!(self.frames[fi].closure.get(fv_idx), Some(c) if matches!(&*c.borrow(), PyObject::Cell { .. }));
                    if existing_is_cell {
                        let cell = self.frames[fi].closure[fv_idx].clone();
                        let mut cell_val = cell.borrow_mut();
                        if let PyObject::Cell { value } = &mut *cell_val {
                            *value = Some(val);
                        }
                    } else {
                        return Err(PyError::name_error(
                            format!("variable '{}' not found", 
                                self.frames[fi].code.freevars.get(fv_idx).map(|s| s.as_str()).unwrap_or("?"))
                        ));
                    }
                }
            }

            Opcode::DELETE_FAST => {
                let var_idx = arg as usize;
                let name = self.frames[fi].code.varnames[var_idx].to_string();
                self.frames[fi].remove_local(&name);
            }

            Opcode::DELETE_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names[name_idx].to_string();
                if let Some(live_module) = self.frames[fi].live_module.clone() {
                    if let PyObject::Module { dict, .. } = &mut *live_module.borrow_mut() {
                        dict.remove(&interner::intern(&name));
                    }
                }
                self.frames[fi].globals.borrow_mut().remove(&interner::intern(&name));
            }

            Opcode::POP_TOP => {
                self.frames[fi].pop()?;
            }

            Opcode::DUP_TOP => {
                let val = self.frames[fi].peek(0)?;
                self.frames[fi].push(val);
            }

            Opcode::COPY => {
                let depth = arg as usize;
                if depth >= self.frames[fi].stack.len() {
                    // Graceful fallback: if depth exceeds stack, treat as DUP_TOP
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

            Opcode::RETURN_VALUE => {
                let val = self.frames[fi].pop()?;
                return Ok(Some(val));
            }

            // ── Unimplemented opcode stubs ────────────────────────────
            Opcode::GET_LEN => {
                let obj = self.frames[fi].pop()?;
                let len = crate::object::builtin_len(&[obj])?;
                self.frames[fi].push(len);
            }
            Opcode::MATCH_MAPPING => {
                let subject = self.frames[fi].peek(0)?;
                let is_map = matches!(&*subject.borrow(), PyObject::Dict(_) | PyObject::Instance { .. });
                self.frames[fi].push(py_bool(is_map));
            }
            Opcode::MATCH_SEQUENCE => {
                let subject = self.frames[fi].peek(0)?;
                let is_seq = matches!(&*subject.borrow(), PyObject::List(_) | PyObject::Tuple(_) | PyObject::Str(_) | PyObject::Bytes(_) | PyObject::ByteArray(_));
                self.frames[fi].push(py_bool(is_seq));
            }
            Opcode::MATCH_KEYS => {
                let _keys = self.frames[fi].pop()?;
                // Simplified: always succeed for dict pattern matching
                self.frames[fi].push(py_bool(true));
            }
            Opcode::CALL_INTRINSIC_1 => {
                let intrinsic = arg;
                match intrinsic {
                    1 => { // INTRINSIC_1_INVALIDATION_COUNTER
                        self.frames[fi].push(py_int(0));
                    }
                    2 => { // INTRINSIC_1_PRINT
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
                // Intrinsics for mutable keys, etc.
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
                    // Fall back to unpack protocol
                    let it = crate::object::builtin_iter(&[seq.clone()])?;
                    let v1 = crate::object::builtin_next(&[it.clone()])?;
                    let v2 = crate::object::builtin_next(&[it.clone()])?;
                    self.frames[fi].push(v1);
                    self.frames[fi].push(v2);
                }
            }

            // ── Register-based instructions ─────────────────────────
            Opcode::REG_MOV => {
                // Lazily initialize registers
                if self.frames[fi].registers.is_empty() {
                    self.frames[fi].registers = Box::new(vec![None; 256]);
                }
                let dst = (arg >> 4) as usize;
                let src = (arg & 0xF) as usize;
                let val = self.frames[fi].registers[src].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_MOV: source register is empty"))?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(val);
                }
            }
            Opcode::REG_LOAD_CONST => {
                let dst = (arg >> 4) as usize;
                let const_idx = (arg & 0xFF) as usize;
                let const_val = self.frames[fi].code.consts.get(const_idx).ok_or_else(|| {
                    PyError::runtime_error("REG_LOAD_CONST: index out of range")
                })?.clone();
                let obj = match const_val {
                    ConstValue::None => py_none(),
                    ConstValue::Bool(b) => py_bool(b),
                    ConstValue::Int(s) => {
                        if let Ok(n) = s.parse::<i64>() { py_int(n) }
                        else { let n: BigInt = s.parse().map_err(|_| PyError::value_error("invalid int"))?; PyObjectRef::imm(PyObject::Int(n)) }
                    }
                    ConstValue::Float(s) => py_float(s.parse().map_err(|_| PyError::value_error("invalid float"))?),
                    ConstValue::String(s) => py_str(&s),
                    ConstValue::Bytes(b) => PyObjectRef::imm(PyObject::Bytes(b)),
                    ConstValue::Complex { real, imag } => {
                        let re: f64 = real.parse().map_err(|_| PyError::value_error("invalid complex literal"))?;
                        let im: f64 = imag.parse().map_err(|_| PyError::value_error("invalid complex literal"))?;
                        PyObjectRef::imm(PyObject::Complex(re, im))
                    }
                    ConstValue::Code(code) => PyObjectRef::imm(PyObject::Code(Rc::from(code))),
                    ConstValue::Tuple(items) => {
                        let objs: Vec<PyObjectRef> = items.into_iter().map(|s| py_str(&s)).collect();
                        PyObjectRef::imm(PyObject::Tuple(objs))
                    }
                };
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(obj);
                }
            }
            Opcode::REG_LOAD_FAST => {
                let dst = (arg >> 4) as usize;
                let var_idx = (arg & 0xFF) as usize;
                let val = self.frames[fi].fast_locals.get(var_idx).and_then(|v| v.clone())
                    .ok_or_else(|| PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                        self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s)))))?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(val);
                }
            }
            Opcode::REG_STORE_FAST => {
                let src = (arg >> 4) as usize;
                let var_idx = (arg & 0xFF) as usize;
                let val = self.frames[fi].registers[src].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_STORE_FAST: source register is empty"))?;
                if var_idx < self.frames[fi].fast_locals.len() {
                    self.frames[fi].fast_locals[var_idx] = Some(val.clone());
                }
                let name = Some(crate::interner::lookup_str(self.frames[fi].code.varnames[var_idx])).ok_or_else(|| {
                    PyError::runtime_error("varname index out of range")
                })?.clone();
                self.frames[fi].insert_local(&name, val);
            }
            Opcode::REG_BINARY_OP => {
                let dst = (arg >> 4) as usize;
                let a_reg = ((arg >> 2) & 0x3) as usize;
                let b_reg = (arg & 0x3) as usize;
                let op = (arg >> 8) as u32;
                let a = self.frames[fi].registers[a_reg].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_BINARY_OP: a is empty"))?;
                let b = self.frames[fi].registers[b_reg].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_BINARY_OP: b is empty"))?;
                let result = match op {
                    0 => py_add(&a, &b),
                    1 => py_sub(&a, &b),
                    2 => py_mul(&a, &b),
                    3 => py_div(&a, &b),
                    4 => py_floor_div(&a, &b),
                    5 => py_mod(&a, &b),
                    6 => py_pow(&a, &b),
                    7 => py_lshift(&a, &b),
                    8 => py_rshift(&a, &b),
                    9 => py_bit_or(&a, &b),
                    10 => py_bit_xor(&a, &b),
                    11 => py_bit_and(&a, &b),
                    13 => py_getitem(&a, &b),
                    _ => return Err(PyError::runtime_error(format!("unknown reg binary op: {}", op))),
                }?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(result);
                }
            }
            Opcode::REG_LOAD_GLOBAL => {
                let dst = (arg >> 4) as usize;
                let name_idx = (arg & 0xFF) as usize;
                let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                // Check inline cache first
                let instr_ip = self.frames[fi].ip - 1;
                if let Some(cached) = self.frames[fi].global_cache.get(instr_ip).and_then(|c| c.clone()) {
                    if dst < self.frames[fi].registers.len() {
                        self.frames[fi].registers[dst] = Some(cached);
                    }
                } else {
                    let val = self.frames[fi].globals.borrow().get(&interner::intern(name)).cloned()
                        .or_else(|| self.frames[fi].builtins.get(&interner::intern(name)).cloned());
                    if let Some(v) = val {
                        if instr_ip < self.frames[fi].global_cache.len() {
                            self.frames[fi].global_cache[instr_ip] = Some(v.clone());
                        }
                        if dst < self.frames[fi].registers.len() {
                            self.frames[fi].registers[dst] = Some(v);
                        }
                    } else {
                        return Err(PyError::name_error(format!("name '{}' is not defined", name)));
                    }
                }
            }
            Opcode::REG_RETURN => {
                let src = (arg & 0xFF) as usize;
                let val = self.frames[fi].registers[src].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_RETURN: register is empty"))?;
                return Ok(Some(val));
            }
            Opcode::REG_BUILD_LIST => {
                // arg: upper 4 bits = dst, lower 4 bits = count
                let dst = (arg >> 4) as usize;
                let count = (arg & 0xF) as usize;
                let mut items = Vec::with_capacity(count);
                for i in 0..count {
                    if let Some(val) = self.frames[fi].registers[i].clone() {
                        items.push(val);
                    }
                }
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(py_list(items));
                }
            }

            Opcode::PUSH_NULL => {
                self.frames[fi].push(py_none());
            }

            Opcode::CALL => {
                let npos = arg as usize & 0xFF;
                let nkw = (arg as usize >> 8) & 0xFF;
                // Pop only the items for THIS call, not the entire stack.
                // The stack has: [callable, arg1, ..., argN, kw1_name, kw1_val, ..., or **kwargs_dict]
                // Total items to pop: npos positional + up to 2*nkw keyword items + 1 callable
                // But **kwargs pushes only 1 item (the dict), not 2.
                // We pop npos + 2*nkw items (generous upper bound) then the callable.
                // The keyword scanner below handles both named kws (2 items) and **kwargs (1 item).
                let total_to_pop = npos + 2 * nkw;
                let mut items = Vec::with_capacity(total_to_pop);
                for _ in 0..total_to_pop {
                    if self.frames[fi].stack.len() > 1 {
                        items.push(self.frames[fi].pop()?);
                    } else {
                        break;
                    }
                }
                let callable = self.frames[fi].pop()?;
                items.reverse();
                // Separate positional args and keywords
                let mut args = Vec::new();
                let mut keywords = Vec::new();
                let mut i = 0;
                // Use npos to determine positional args count
                while i < npos && i < items.len() {
                    args.push(items[i].clone());
                    i += 1;
                }
                // Remaining items are keyword name+value pairs or **kwargs dict
                while i + 1 < items.len() {
                    if let PyObject::Str(name) = &*items[i].borrow() {
                        keywords.push((name.to_string(), items[i+1].clone()));
                        i += 2;
                    } else {
                        // **kwargs dict or packed arg
                        break;
                    }
                }
                let result = self.call_function(callable, args, keywords)?;
                self.frames[fi].push(result);
            }

            Opcode::MAKE_CELL => {
                let idx = arg as usize;
                let frame = &mut self.frames[fi];
                if idx < frame.fast_locals.len() {
                    let val = frame.fast_locals[idx].take();
                    let cell = PyObjectRef::new(PyObject::Cell { value: val });
                    frame.fast_locals[idx] = Some(cell);
                }
            }

            Opcode::COPY_FREE_VARS => {
                let nfree = arg as usize;
                let mut cells = Vec::with_capacity(nfree);
                for _ in 0..nfree {
                    cells.push(self.frames[fi].pop()?);
                }
                // Store the closure tuple on the stack for MAKE_FUNCTION to consume
                self.frames[fi].push(PyObjectRef::imm(PyObject::Tuple(cells)));
            }

            Opcode::MAKE_FUNCTION => {
                let has_closure = (arg & 0x100) != 0;
                let n_defaults = (arg & 0xFF) as usize;
                let n_kwdefaults = ((arg >> 9) & 0xFF) as usize;
                // Stack (bottom to top): [closure?, CODE, pos_defaults...,
                // kwonly_defaults...] — kwonly defaults were pushed last, so
                // pop them first. Appended after positional defaults in the
                // final `defaults` vec (see CodeObject::kwonly_defaults_mask
                // for how call-binding tells the two apart).
                let mut kwdefaults = Vec::new();
                for _ in 0..n_kwdefaults {
                    kwdefaults.push(self.frames[fi].pop()?);
                }
                kwdefaults.reverse();
                let mut defaults = Vec::new();
                for _ in 0..n_defaults {
                    defaults.push(self.frames[fi].pop()?);
                }
                defaults.reverse();
                defaults.extend(kwdefaults);
                let code_obj = self.frames[fi].pop()?;
                // A cheap `Rc` clone, not a deep copy of the whole
                // `CodeObject` (instructions, consts, ...) — this used to
                // `.clone()` the dereferenced `CodeObject` itself here,
                // meaning a `def`/`lambda` executed fresh on every
                // iteration of a loop deep-cloned its entire compiled body
                // every single time, even though (with `LOAD_CONST`'s own
                // caching) the SAME `PyObject::Code` constant was being
                // read repeatedly.
                let code = match &*code_obj.borrow() {
                    PyObject::Code(c) => c.clone(),
                    _ => return Err(PyError::runtime_error("MAKE_FUNCTION: expected code object")),
                };
                let closure = if has_closure {
                    let closure_tuple = self.frames[fi].pop()?;
                    let items = closure_tuple.borrow();
                    if let PyObject::Tuple(items) = &*items {
                        items.clone()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                // Use module_globals when available (class body execution) so that
                // functions defined inside a class body capture the module's globals
                // (e.g. 'empty' from django.utils.functional) rather than the class
                // namespace. Falls back to the frame's globals for module-level code
                // and regular function calls.
                let globals = self.frames[fi].module_globals.clone()
                    .unwrap_or_else(|| self.frames[fi].globals.clone());
                let code_obj = code.clone();
                let func = PyObjectRef::new(PyObject::Function(Box::new(PyFunction {
                    code: code_obj.clone(),
                    globals,
                    defaults,
                    closure,
                    dict: HashMap::new(),
                    jit_ptr: std::cell::Cell::new(0),
                    jit_consts: std::cell::RefCell::new(Vec::new()),
                })));
                // Set __code__ and __module__ on the function
                if let PyObject::Function(ref mut inner_f) = &mut *func.borrow_mut() {
                let dict = &mut inner_f.dict;
                    dict.insert_str("__code__", PyObjectRef::imm(PyObject::Code(code_obj)));
                }
                if let Some(ref mg) = self.frames[fi].module_globals {
                    let mg = mg.borrow();
                    if let Some(module_name) = mg.get(&interner::intern("__name__")) {
                        if let PyObject::Str(s) = &*module_name.borrow() {
                            if let PyObject::Function(ref mut inner_f) = &mut *func.borrow_mut() {
                let dict = &mut inner_f.dict;
                                dict.insert_str("__module__", py_str(s));
                            }
                        }
                    }
                }
                self.frames[fi].push(func);
            }

            Opcode::BUILD_LIST => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(py_list(items));
            }

            Opcode::BUILD_TUPLE => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(py_tuple(items));
            }

            Opcode::BUILD_MAP => {
                self.frames[fi].push(py_dict());
            }

            Opcode::BUILD_SET => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(PyObjectRef::new(PyObject::Set(PySet::from_vec(items)?)));
            }

            Opcode::BUILD_STRING => {
                let count = arg as usize;
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(self.frames[fi].pop()?.str());
                }
                parts.reverse();
                self.frames[fi].push(py_str(&parts.join("")));
            }

            Opcode::BUILD_SLICE => {
                let nargs = arg as usize;
                let step = if nargs >= 3 { Some(self.frames[fi].pop()?) } else { None };
                let stop = if nargs >= 2 { Some(self.frames[fi].pop()?) } else { None };
                let start = if nargs >= 1 { Some(self.frames[fi].pop()?) } else { None };
                self.frames[fi].push(PyObjectRef::imm(PyObject::Slice {
                    start: start.unwrap_or(py_none()),
                    stop: stop.unwrap_or(py_none()),
                    step: step.unwrap_or(py_none()),
                }));
            }

            Opcode::BINARY_OP => {
                // `arg >= 100` encodes the IN-PLACE variant of operator
                // `arg - 100` (`x += y` etc. — `Stmt::AugAssign`'s codegen
                // is the only emitter of this range). Try `__iadd__`/
                // `__isub__`/etc. first (only meaningful for a
                // `PyObject::Instance` with such a dunder defined — every
                // native type falls through unchanged), then fall back to
                // the exact same logic as the plain, non-augmented operator
                // below. Previously `x += y` NEVER checked for `__iadd__`
                // at all (AugAssign compiled to a bare `BINARY_OP` with the
                // SAME arg as `x + y`) — `__iadd__`'s entire purpose (an
                // object choosing to mutate itself and return `self`,
                // instead of `__add__`'s always-build-a-new-object
                // semantics) was silently unreachable for every user class
                // in the interpreter's history. Confirmed general via
                // CPython's own `test_augassign.py`.
                let (op, in_place) = if arg >= 100 { (arg - 100, true) } else { (arg, false) };
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                if in_place {
                    let idunder = match op {
                        0 => Some("__iadd__"), 1 => Some("__isub__"), 2 => Some("__imul__"),
                        3 => Some("__itruediv__"), 4 => Some("__ifloordiv__"), 5 => Some("__imod__"),
                        6 => Some("__ipow__"), 7 => Some("__ilshift__"), 8 => Some("__irshift__"),
                        9 => Some("__ior__"), 10 => Some("__ixor__"), 11 => Some("__iand__"),
                        12 => Some("__imatmul__"), _ => None,
                    };
                if let Some(name) = idunder {
                    if matches!(&*left.borrow(), PyObject::Instance { .. }) {
                        if let Some(r) = crate::object::try_dunder_binop(&left, &right, name)? {
                            self.frames[fi].push(r);
                            return Ok(None);
                        }
                    }
                }
                }
                // Native `deque` has no real Python-callable `__iadd__`/
                // `__imul__` dunder in its type dict (native methods are
                // dispatched via `attrs.rs`'s `get_attribute_impl`, which
                // doesn't fire for operator opcodes) — so `d += 'bcd'` /
                // `d *= 3` on a raw deque would otherwise fall through to
                // `py_add`/`py_mul` below, which are correct for `d + e`/
                // `d * n` (both build a NEW deque) but wrong for the
                // in-place forms (`d += 'bcd'` must EXTEND the live deque
                // even though `d + 'bcd'` raises TypeError). Handle the
                // in-place forms directly here.
                if in_place {
                    let is_deque = matches!(&*left.borrow(), PyObject::Deque { .. });
                    if is_deque {
                        match op {
                            // `d += iterable` — extend in place (real
                            // CPython's `deque.__iadd__`), accepts any
                            // iterable. Materialize the source FIRST so
                            // self-extend (`d += d`) doesn't trip the deque
                            // iterator's own mutation detection mid-iteration.
                            0 => {
                                let it = crate::object::builtin_iter(&[right])?;
                                let mut items = Vec::new();
                                loop {
                                    match crate::object::builtin_next(&[it.clone()]) {
                                        Ok(v) => items.push(v),
                                        Err(crate::object::PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                                {
                                    if let PyObject::Deque { data, maxlen } = &mut *left.borrow_mut() {
                                        for item in items {
                                            data.push_back(item);
                                            if let Some(maxlen) = maxlen {
                                                while data.len() > *maxlen { data.pop_front(); }
                                            }
                                        }
                                    }
                                }
                                self.frames[fi].push(left);
                                return Ok(None);
                            }
                            // `d *= n` — repeat in place, truncated to maxlen.
                            2 => {
                                let n = right.as_i64().ok_or_else(|| PyError::type_error("an integer is required"))?;
                                if let PyObject::Deque { data, maxlen } = &mut *left.borrow_mut() {
                                    let n = n.max(0) as usize;
                                    let items: Vec<crate::object::PyObjectRef> = data.iter().cloned().collect();
                                    data.clear();
                                    for _ in 0..n {
                                        for item in &items {
                                            data.push_back(item.clone());
                                            if let Some(maxlen) = maxlen {
                                                while data.len() > *maxlen { data.pop_front(); }
                                            }
                                        }
                                    }
                                }
                                self.frames[fi].push(left);
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                }
                let result = match op {
                    0 => py_add(&left, &right),
                    1 => py_sub(&left, &right),
                    2 => py_mul(&left, &right),
                    3 => py_div(&left, &right),
                    4 => py_floor_div(&left, &right),
                    5 => py_mod(&left, &right),
                    6 => py_pow(&left, &right),
                    7 => py_lshift(&left, &right),
                    8 => py_rshift(&left, &right),
                    9 => py_bit_or(&left, &right),
                    10 => py_bit_xor(&left, &right),
                     11 => py_bit_and(&left, &right),
                     12 => {
                         (|| -> PyResult<PyObjectRef> {
                             if let Some(r) = crate::object::try_dunder_binop(&left, &right, "__matmul__")? {
                                 return Ok(r);
                             }
                             if let Some(r) = crate::object::try_dunder_binop(&right, &left, "__rmatmul__")? {
                                 return Ok(r);
                             }
                             Err(PyError::type_error(format!("unsupported operand type(s) for @: '{}' and '{}'",
                                 left.borrow().type_name(), right.borrow().type_name())))
                         })()
                     }
                     13 => py_getitem(&left, &right),
                     _ => return Err(PyError::runtime_error(format!("unknown binary op: {}", op))),
                }?;
                self.frames[fi].push(result);
            }

            Opcode::COMPARE_OP => {
                let op = arg;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let result = py_compare(&left, &right, op)?;
                self.frames[fi].push(result);
            }

            Opcode::IS_OP => {
                let invert = arg != 0;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let is_same = left.is(&right);
                let result = if invert { !is_same } else { is_same };
                self.frames[fi].push(py_bool(result));
            }

            Opcode::CONTAINS_OP => {
                let invert = arg != 0;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let result = contains_op(&right, &left)?;
                let result = if invert { !result } else { result };
                self.frames[fi].push(py_bool(result));
            }

            Opcode::UNARY_NEGATIVE => {
                let val = self.frames[fi].pop()?;
                // Custom classes with __neg__ (e.g. Decimal) need it invoked
                // directly — implementing this as `0 - val` only works if
                // int.__sub__ knows how to handle an arbitrary Instance
                // operand via reflection, which try_dunder_binop doesn't do
                // (it only ever checks the left operand's own dunder).
                let neg_method = if let PyObject::Instance { typ, .. } = &*val.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__neg__")
                } else {
                    None
                };
                let result = if let Some(f) = neg_method {
                    call_bound_method(f, val.clone(), vec![])?
                } else {
                    py_sub(&py_int(0), &val)?
                };
                self.frames[fi].push(result);
            }

            Opcode::UNARY_POSITIVE => {
                let val = self.frames[fi].pop()?;
                let pos_method = if let PyObject::Instance { typ, .. } = &*val.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__pos__")
                } else {
                    None
                };
                let result = if let Some(f) = pos_method {
                    call_bound_method(f, val.clone(), vec![])?
                } else {
                    py_pos(&val)?
                };
                self.frames[fi].push(result);
            }

            Opcode::UNARY_NOT => {
                let val = self.frames[fi].pop()?;
                self.frames[fi].push(py_bool(!val.truthy()));
            }

            Opcode::UNARY_INVERT => {
                let val = self.frames[fi].pop()?;
                let result = {
                    let obj = val.borrow();
                    match &*obj {
                        PyObject::Int(i) => py_int(!i),
                        _ => return Err(PyError::type_error("bad operand type for unary ~")),
                    }
                };
                self.frames[fi].push(result);
            }

            Opcode::JUMP_FORWARD | Opcode::JUMP | Opcode::JUMP_BACKWARD => {
                let offset = arg as usize;
        match op {
                    Opcode::JUMP_FORWARD => {
                        self.frames[fi].ip += offset;
                    }
                    Opcode::JUMP => {
                        self.frames[fi].ip = offset;
                    }
                    Opcode::JUMP_BACKWARD => {
                        let cur_ip = self.frames[fi].ip;
                        self.frames[fi].ip = cur_ip.wrapping_sub(offset).wrapping_sub(1);
                    }
                    _ => unreachable!(),
                }
            }

            Opcode::POP_JUMP_IF_FALSE => {
                let val = self.frames[fi].pop()?;
                if !val.truthy() {
                    self.frames[fi].ip = arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_TRUE => {
                let val = self.frames[fi].pop()?;
                if val.truthy() {
                    self.frames[fi].ip = arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_NONE => {
                let val = self.frames[fi].pop()?;
                let is_none = {
                    matches!(&*val.borrow(), PyObject::None)
                };
                if is_none {
                    self.frames[fi].ip = arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_NOT_NONE => {
                let val = self.frames[fi].pop()?;
                let is_not_none = {
                    !matches!(&*val.borrow(), PyObject::None)
                };
                if is_not_none {
                    self.frames[fi].ip = arg as usize;
                }
            }

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
                    } else { false };
                    if !has_override {
                        if let Some(native) = crate::object::native_backing_of(&val) {
                            let iterator = crate::object::builtin_iter(&[native])?;
                            self.frames[fi].push(iterator);
                            return Ok(None);
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
                        return Ok(None);
                    }
                    use crate::object::ObjectAccess;
                    let raw_method = val.borrow().get_attribute("__iter__")
                        .map_err(|_| PyError::type_error(format!("'{}' object is not iterable", val.borrow().type_name())))?;
                    let val_clone = val.clone();
                    let iter_method = PyObjectRef::imm(PyObject::BoundMethod {
                        func: raw_method,
                        self_obj: val_clone,
                    });
                    let iterator = self.call_function(iter_method, vec![], vec![])?;
                    // Eagerly consume via builtin_next(), which — unlike a raw
                    // get_attribute("__next__") — correctly handles both a
                    // user Instance with its own __next__ AND a native iterator
                    // (e.g. ListIter) that __iter__ delegated to, such as
                    // `def __iter__(self): return iter(self.data)`.
                    let mut items: Vec<PyObjectRef> = Vec::new();
                    loop {
                        match crate::object::builtin_next(&[iterator.clone()]) {
                            Ok(val) => items.push(val),
                            Err(PyError::StopIteration) => break,
                            Err(e) => return Err(e),
                        }
                    }
                    self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: items, index: 0 }));
                } else {
                let obj = val.borrow();
                match &*obj {
                    PyObject::List(v) => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: v.clone(), index: 0 }));
                    }
                    PyObject::Deque { data, .. } => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::DequeIter { deque: val.clone(), index: 0, start_len: data.len() }));
                    }
                    PyObject::Tuple(v) => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: v.clone(), index: 0 }));
                    }
                    PyObject::Str(s) => {
                        let chars: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: chars, index: 0 }));
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
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: s.to_vec(), index: 0 }));
                    }
                    PyObject::Bytes(b) => {
                        let items: Vec<PyObjectRef> = b.iter().map(|byte| py_int(*byte as i64)).collect();
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: items, index: 0 }));
                    }
                    PyObject::ByteArray(b) => {
                        let items: Vec<PyObjectRef> = b.iter().map(|byte| py_int(*byte as i64)).collect();
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: items, index: 0 }));
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
                        self.frames[fi].push(PyObjectRef::new(PyObject::RangeIter { current: *start, stop: *stop, step: *step }));
                    }
                    PyObject::Dict(ref pydict) => {
                        let keys: Vec<PyObjectRef> = pydict.keys();
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: keys, index: 0 }));
                    }
                    PyObject::EnumerateIter { .. } => {
                        drop(obj);
                        self.frames[fi].push(val);
                    }
                    // Iterators are their own iterator (matching CPython's
                    // `__iter__` returning self) — `for x in iter(y):` or
                    // `for x in itertools.tee(y)[0]:` must work the same as
                    // iterating the original iterable directly.
                    PyObject::ListIter { .. } | PyObject::RangeIter { .. }
                    | PyObject::MapIterator { .. } | PyObject::FilterIterator { .. }
                    | PyObject::ZipIterator { .. } | PyObject::CycleIter { .. }
                    | PyObject::GroupByIter { .. } | PyObject::GetItemIter { .. }
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
                                let iterator = self.call_function(f, vec![val.clone()], vec![])?;
                                self.frames[fi].push(iterator);
                            }
                            None => return Err(PyError::type_error(format!("'{}' object is not iterable", val.get_type_name()))),
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
                        file.borrow_mut().read_to_end(&mut rest).map_err(|e| PyError::os_error_from_io(&e))?;
                        drop(obj);
                        let mut lines: Vec<PyObjectRef> = Vec::new();
                        let mut current: Vec<u8> = Vec::new();
                        for byte in rest {
                            current.push(byte);
                            if byte == b'\n' {
                                lines.push(if binary { PyObjectRef::imm(PyObject::Bytes(current.clone())) } else { py_str(&String::from_utf8_lossy(&current)) });
                                current.clear();
                            }
                        }
                        if !current.is_empty() {
                            lines.push(if binary { PyObjectRef::imm(PyObject::Bytes(current.clone())) } else { py_str(&String::from_utf8_lossy(&current)) });
                        }
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: lines, index: 0 }));
                    }
                    _ => return Err(PyError::type_error(format!("'{}' object is not iterable", obj.type_name()))),
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
                            } else { return Err(PyError::runtime_error("expected __next__ method")) }
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
                        PyObject::RangeIter { current, stop, step } => {
                            if *step > 0 { *current >= *stop } else { *current <= *stop }
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
                        PyObject::ZipIterator { .. } | PyObject::MapIterator { .. } | PyObject::FilterIterator { .. } | PyObject::CycleIter { .. } | PyObject::EnumerateIter { .. } | PyObject::GroupByIter { .. } | PyObject::GetItemIter { .. } | PyObject::CallSentinelIter { .. } | PyObject::DequeIter { .. } => {
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
                            return Ok(None);
                        }
                        _ => {
                            // Not a built-in iterator — check for __next__ protocol
                            if obj.type_name() == "instance" {
                                return self.for_iter_next(iter_val.clone(), arg);
                            }
                            return Err(PyError::type_error("for_iter on non-iterable"))
                        },
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
                                if let PyObject::List(v) = &*obj { v.clone() } else { unreachable!() }
                            };
                            *val.borrow_mut() = PyObject::ListIter { list: list_clone, index: 0 };
                        }
                        let mut obj = val.borrow_mut();
                        match &mut *obj {
                            PyObject::ListIter { list, index } => {
                                let v = list[*index].clone();
                                *index += 1;
                                v
                            }
                            PyObject::RangeIter { current, stop: _, step } => {
                                let v = py_int(*current);
                                // See the matching fix in `object.rs`'s
                                // `builtin_next` `RangeIter` arm — plain
                                // `+=` panics near i64::MAX/MIN.
                                *current = current.checked_add(*step).unwrap_or(if *step > 0 { i64::MAX } else { i64::MIN });
                                v
                            }
                            // `EnumerateIter` no longer reaches this arm at
                            // all — it moved to the earlier "delegate to
                            // builtin_next, return early" bucket above
                            // (alongside Zip/Map/Filter/Cycle) once it
                            // became a lazy `source`-wrapper instead of a
                            // materialized `items` list with no `len()` to
                            // compare against.
                            _ => unreachable!()
                        }
                    };
                    self.frames[fi].push(val);
                    self.frames[fi].push(item);
                }
                }
            }

            Opcode::LOAD_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let obj = self.frames[fi].pop()?;
                let result = {
                    let obj_borrowed = obj.borrow();
                    match &*obj_borrowed {
                        // `it.__next__()`/`it.__iter__()` on any of this
                        // codebase's iterator shapes: `attrs.rs`'s
                        // `get_attribute_impl` (called from `&PyObject`,
                        // with no access to the enclosing `PyObjectRef`)
                        // can only bind a FRESH CLONE of the iterator as
                        // `self_obj` — for a stateful iterator (advancing
                        // via a mutable `index` field), that silently
                        // disconnects the returned method from the real
                        // object: every call to `it.__next__()` re-read the
                        // SAME starting state instead of advancing
                        // (confirmed via direct repro: three successive
                        // `it.__next__()` calls all returned the first
                        // element). Handled here instead, where the real
                        // `obj` `PyObjectRef` is available to bind directly
                        // — same fix shape as `GET_AWAITABLE`'s own
                        // `self_obj`-rebind elsewhere in this file, for the
                        // identical underlying limitation.
                        PyObject::ListIter { .. } | PyObject::RangeIter { .. } | PyObject::MapIterator { .. }
                        | PyObject::FilterIterator { .. } | PyObject::ZipIterator { .. } | PyObject::CycleIter { .. }
                        | PyObject::GroupByIter { .. } | PyObject::EnumerateIter { .. } | PyObject::GetItemIter { .. }
                        | PyObject::CallSentinelIter { .. } if name == "__next__" || name == "__iter__" => {
                            let func: crate::object::BuiltinFunc = if name == "__next__" {
                                crate::object::builtin_next
                            } else {
                                crate::object::builtin_iter
                            };
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func,
                                self_obj: obj.clone(),
                            }))
                        }
                        // `range_iterator`/`list_iterator.__setstate__(state)`
                        // (real CPython's pickle-restore protocol, also
                        // directly usable) — needs the same real-`self_obj`
                        // treatment as `__next__`/`__iter__` just above,
                        // since it MUTATES the iterator's position in place
                        // (a disconnected clone would silently do nothing).
                        // Found via CPython's own `test_range.py::
                        // test_iterator_setstate`.
                        PyObject::RangeIter { .. } if name == "__setstate__" => {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: crate::object::range_iter_setstate,
                                self_obj: obj.clone(),
                            }))
                        }
                        PyObject::ListIter { .. } if name == "__setstate__" => {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: crate::object::list_iter_setstate,
                                self_obj: obj.clone(),
                            }))
                        }
                        PyObject::Super { cls: _, obj: _super_obj } => {
                            // super(cls, obj).attr: walk MRO of obj's type, starting after cls
                            drop(obj_borrowed);
                            let attr = obj.borrow().get_attribute(&name)?;
                            Ok(attr)
                        }
                        PyObject::Instance { dict, typ } => {
                            // Inline attribute cache: skip full lookup if cached
                            // with matching type tag — only valid when this
                            // instance's OWN dict doesn't also define `name`,
                            // since the cache only ever stores type/mro-level
                            // hits (methods, class attributes); an instance
                            // that shadows the class attribute with its own
                            // instance-level value of the same name must still
                            // win over a stale cache entry from some OTHER
                            // instance of the same type that didn't have that
                            // override (see the caching-site comment below for
                            // the matching write-side half of this fix).
                            let type_tag = typ.get_id() as u64;
                            let cached = if dict.contains_key(&name) { None } else {
                                self.frames[fi].attr_cache.get(name_idx)
                                    .and_then(|entry| entry.as_ref())
                                    .filter(|(tag, _)| *tag == type_tag)
                                    .map(|(_, val)| val.clone())
                            };
                            if let Some(cached_val) = cached {
                                // The cached value may be a method already
                                // BOUND to whatever instance first populated
                                // this cache slot (`self_obj` baked in) — the
                                // cache itself is keyed only by
                                // `(name_idx, type_tag)`, with no per-
                                // instance component, so reusing it AS-IS
                                // for a DIFFERENT instance of the same type
                                // silently operated on the wrong `self`
                                // (confirmed via a direct repro: calling the
                                // same bound-method-shaped attribute on two
                                // different instances of the same class
                                // within one frame — e.g.
                                // `subprocess.CompletedProcess.check_returncode`
                                // — the second call silently used the
                                // FIRST instance as `self`). Rebind to the
                                // CURRENT `obj` before returning, matching
                                // the same rebind-on-hit fix already applied
                                // to the OTHER (module-level) attribute
                                // cache just above in this file.
                                //
                                // Only rebind when the cached `self_obj` is
                                // ITSELF an `Instance` of this SAME type —
                                // i.e. unambiguously "some OTHER instance of
                                // the identical class", the exact cross-
                                // instance-pollution case above. A cached
                                // method deliberately bound to something
                                // ELSE (native-backing delegation for a
                                // class transparently subclassing list/
                                // dict/str, or any other legitimate "bound
                                // to a fixed, different object" case) must
                                // be returned UNCHANGED — rebinding it
                                // unconditionally to `obj` broke exactly
                                // that (confirmed regression: `collections`'
                                // own `Counter.update` internals, which rely
                                // on a cached method staying bound to its
                                // real native-backing dict rather than the
                                // wrapper `Instance`).
                                let rebound = match &*cached_val.borrow() {
                                    PyObject::BuiltinMethod { name: n, func, self_obj }
                                        if matches!(&*self_obj.borrow(), PyObject::Instance { typ: t, .. } if t.is(typ)) =>
                                    {
                                        PyObjectRef::imm(PyObject::BuiltinMethod { name: n.clone(), func: *func, self_obj: obj.clone() })
                                    }
                                    PyObject::BoundMethod { func, self_obj }
                                        if matches!(&*self_obj.borrow(), PyObject::Instance { typ: t, .. } if t.is(typ)) =>
                                    {
                                        PyObjectRef::imm(PyObject::BoundMethod { func: func.clone(), self_obj: obj.clone() })
                                    }
                                    _ => cached_val.clone(),
                                };
                                self.frames[fi].push(rebound);
                                return Ok(None);
                            }
                            if name == "__dict__" {
                                // Return a live Dict view backed by the instance's HashMap.
                                // NATIVE_BACKING_KEY is internal bookkeeping
                                // (see native_backing_of) and must not leak
                                // into user-visible introspection.
                                let mut pd = crate::object::PyDict::new();
                                for (k, v) in dict.iter() {
                                    if k == crate::object::NATIVE_BACKING_KEY { continue; }
                                    let key = py_str(k);
                                    pd.set(key, v.clone())?;
                                }
                                drop(obj_borrowed);
                                pd.instance_ref = Some(obj.clone());
                                self.frames[fi].push(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                                return Ok(None);
                            }
                            if name == "__class__" {
                                let cls = typ.clone();
                                drop(obj_borrowed);
                                self.frames[fi].push(cls);
                                return Ok(None);
                            }
                            // Clone dict/typ into owned values and drop the
                            // borrow of `obj` ITSELF now — the descriptor
                            // dispatch below may call into arbitrary Python
                            // code (a `@property` getter, `cached_property`'s
                            // `__get__`, etc.), and such code very commonly
                            // writes back into `instance.__dict__` (that's
                            // literally what `cached_property.__get__` does,
                            // to cache its computed value for next time) —
                            // if `obj`'s own borrow were still held here, that
                            // nested write's borrow_mut() on the SAME RefCell
                            // panics the moment such a getter touches the
                            // instance it was called on (confirmed via a
                            // genuine, general, Django-free repro).
                            let dict: crate::object::AttrMap = dict.clone();
                            let typ: PyObjectRef = typ.clone();
                            drop(obj_borrowed);
                            let dict = &dict;
                            let typ = &typ;
                            let attr = dict.get_str(&name).cloned().or_else(|| {
                                let typ_ref = typ.borrow();
                                if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                                    let found = type_dict.get_str(&name).cloned().or_else(|| {
                                        for base in mro.iter().skip(1) {
                                            if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                                if let Some(val) = base_dict.get_str(&name) {
                                                    return Some(val.clone());
                                                }
                                            }
                                        }
                                        None
                                    });
                                    // Handle descriptor protocol for Property, StaticMethod, ClassMethod, and generic __get__
                                    if let Some(val) = found {
                                        let val_borrowed = val.borrow();
                                        match &*val_borrowed {
                                            PyObject::Property(ref d) if d.getter.is_some() => {
                                                drop(typ_ref);
                                                let g = d.getter.as_ref().unwrap();
                                                return Some(self.call_function(g.clone(), vec![obj.clone()], vec![]).unwrap_or_else(|_| val.clone()));
                                            }
                                            PyObject::StaticMethod { func } => {
                                                return Some(func.clone());
                                            }
                                            PyObject::ClassMethod { func } => {
                                                let func_clone = func.clone();
                                                drop(val_borrowed);
                                                drop(typ_ref);
                                                let cls = obj.borrow();
                                                if let PyObject::Instance { typ: inst_typ, .. } = &*cls {
                                                    // Return a BoundMethod that will prepend the class when called
                                                    let class_obj = inst_typ.clone();
                                                    drop(cls);
                                                    return Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                        func: func_clone,
                                                        self_obj: class_obj,
                                                    }));
                                                }
                                                // When accessing classmethod on a type itself (e.g. MyClass.method),
                                                // bind the type as self so it becomes the first arg on call
                                                let class_obj = obj.clone();
                                                drop(cls);
                                                return Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                    func: func_clone,
                                                    self_obj: class_obj,
                                                }));
                                            }
                                            PyObject::Function(_) => {
                                                let is_instance_obj = matches!(&*obj.borrow(), PyObject::Instance { .. });
                                                if is_instance_obj {
                                                    return Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                        func: val.clone(),
                                                        self_obj: obj.clone(),
                                                    }));
                                                } else {
                                                    return Some(val.clone());
                                                }
                                            }
                                            PyObject::BuiltinFunction { name: n, func }
                                                if crate::object::is_builtin_exception_class_name(n)
                                                    || std::ptr::fn_addr_eq(*func, crate::object::builtin_open as crate::object::BuiltinFunc) =>
                                            {
                                                // Do NOT auto-bind a builtin
                                                // exception "class" (this
                                                // codebase's representation for
                                                // e.g. `AssertionError`) found
                                                // as a plain class attribute
                                                // (`failureException =
                                                // AssertionError`) — unlike a
                                                // genuine native METHOD (also a
                                                // `BuiltinFunction`, e.g.
                                                // `hmac`'s `HMAC.hexdigest`,
                                                // which DOES rely on `self`
                                                // being auto-prepended — see
                                                // the `else` arm just below,
                                                // unchanged for that case), a
                                                // class reference is never a
                                                // descriptor in real Python, so
                                                // binding it here silently
                                                // prepended `self` as an extra
                                                // positional argument to every
                                                // call and turned the class
                                                // reference into a `BoundMethod`
                                                // that `issubclass()` could no
                                                // longer recognize as a class at
                                                // all — confirmed via
                                                // `unittest`'s own
                                                // `self.failureException(msg)`
                                                // (raising `AssertionError(self,
                                                // msg)` instead of
                                                // `AssertionError(msg)`) and
                                                // `issubclass(exc_info[0],
                                                // test.failureException)`
                                                // (always False, misclassifying
                                                // every real test failure as an
                                                // error).
                                                let _ = func;
                                                return Some(val.clone());
                                            }
                                            PyObject::BuiltinFunction { name: n, func } => {
                                                return Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                                    name: n.clone(),
                                                    func: *func,
                                                    self_obj: obj.clone(),
                                                }));
                                            }
                                            PyObject::BuiltinMethod { name: n, func, .. } => {
                                                return Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                                    name: n.clone(),
                                                    func: *func,
                                                    self_obj: obj.clone(),
                                                }));
                                            }
                                            // NOTE: deliberately NOT auto-binding a bare
                                            // `PyObject::Closure` found via the class dict here
                                            // — unlike `Function`/`BuiltinFunction` just above,
                                            // `Closure` is ALSO used pervasively for the
                                            // opposite convention: a closure built FRESH per
                                            // instance (e.g. `io.BytesIO`'s `read`/`write`/
                                            // `seek`, `dev.rs`), capturing that instance's own
                                            // state directly and expecting NO `self` prepended
                                            // at all. Auto-binding unconditionally here broke
                                            // those (their first REAL argument became `self`
                                            // instead) — confirmed via `io.BytesIO().write(b"x")`
                                            // regressing to `TypeError: a bytes-like object is
                                            // required, not str`. A shared, TYPE-level `Closure`
                                            // needing `self` (e.g. `namedtuple`'s own
                                            // `_asdict`/`_replace`) should instead be
                                            // implemented as a plain `BuiltinFunction` reading
                                            // whatever state it needs off `self` at call time —
                                            // that convention already auto-binds correctly via
                                            // the arm above, with no ambiguity.
                                            _ => {
                                                // Generic descriptor protocol: if value has __get__, call it
                                                drop(val_borrowed);
                                                let cls = {
                                                    let owner_type = obj.borrow();
                                                    if let PyObject::Instance { typ: inst_typ, .. } = &*owner_type {
                                                        Some(inst_typ.clone())
                                                    } else {
                                                        None
                                                    }
                                                };
                                                if let Some(cls) = cls {
                                                    if let Ok(__get__) = val.borrow().get_attribute("__get__") {
                                                        if std::env::var("RPY_DEBUG_DESCRIPTOR2").is_ok() {
                                                            eprintln!("INSTANCE-LEVEL __get__: attr_name={} val_type={:?} obj_type={:?}", name, val.borrow().type_name(), obj.borrow().type_name());
                                                        }
                                                        let descriptor_args = vec![val.clone(), obj.clone(), cls];
                                                        match self.call_function(__get__, descriptor_args, vec![]) {
                                                            Ok(v) => return Some(v),
                                                            Err(e) => {
                                                                if std::env::var("RPY_DEBUG_DESCRIPTOR").is_ok() {
                                                                    eprintln!("DESCRIPTOR __get__ FAILED for {:?}: {}", name, e);
                                                                }
                                                                return Some(val.clone());
                                                            }
                                                        }
                                                    }
                                                }
                                                return Some(val.clone());
                                            }
                                        }
                                    }
                                    None
                                } else {
                                    None
                                }
                            });
                            // Not overridden anywhere in the mro: for a class
                            // that transparently subclasses list/dict/str
                            // (`class Foo(list): ...`), delegate to the real
                            // native value's own attribute resolution, rebound
                            // to the native backing (not this instance) since
                            // that's the object whose state actually mutates.
                            // Must run BEFORE the generic dict-like fallback
                            // below, which would otherwise misinterpret the
                            // native backing's own dict entry as plain
                            // instance-attribute data.
                            let attr = attr.or_else(|| {
                                let native = dict.get(crate::object::NATIVE_BACKING_KEY)?;
                                // A deque subclass's `__copy__`/`copy()` must
                                // return a NEW instance of the SAME subclass
                                // (not a raw deque) — build that closure here,
                                // since this inline resolution path mirrors
                                // `get_attribute_impl`'s own handling.
                                if matches!(&*native.borrow(), PyObject::Deque { .. }) && (name == "__copy__" || name == "copy") {
                                    let typ_clone = typ.clone();
                                    let new_native = {
                                        let b = native.borrow();
                                        if let PyObject::Deque { data, maxlen } = &*b {
                                            py_deque(data.clone(), *maxlen)
                                        } else { unreachable!() }
                                    };
                                    return Some(PyObjectRef::new(PyObject::Closure(Rc::new(move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                        let mut new_dict = crate::object::AttrMap::new();
                                        new_dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), new_native.clone());
                                        Ok(PyObjectRef::new(PyObject::Instance { typ: typ_clone.clone(), dict: new_dict }))
                                    }))));
                                }
                                let val = native.borrow().get_attribute(&name).ok()?;
                                let rebound = match &*val.borrow() {
                                    PyObject::BuiltinMethod { name: n, func, .. } => {
                                        Some(PyObjectRef::imm(PyObject::BuiltinMethod { name: n.clone(), func: *func, self_obj: native.clone() }))
                                    }
                                    _ => None,
                                };
                                Some(rebound.unwrap_or(val))
                            });
                            // Fallback for dict methods on dict-derived instances
                            let attr = attr.or_else(|| {
                                if name == "__iter__" || name == "items" || name == "keys" || name == "values" || name == "get" {
                                    let func: crate::object::BuiltinFunc = match name.as_str() {
                                        "__iter__" => crate::object::dict_method_iter,
                                        "items" => crate::object::dict_method_items,
                                        "keys" => crate::object::dict_method_keys,
                                        "values" => crate::object::dict_method_values,
                                        "get" => crate::object::dict_method_get,
                                        _ => return None,
                                    };
                                    Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: name.clone(),
                                        func,
                                        self_obj: obj.clone(),
                                    }))
                                } else {
                                    None
                                }
                            });
                            // PEP 3134 traceback/chaining protocol methods
                            // for a user-defined exception class that
                            // doesn't override them — same fix, same
                            // rationale, as the `get_attribute_impl` copy of
                            // this logic (`object.rs`); this is LOAD_ATTR's
                            // own separate, inline copy of instance
                            // attribute resolution (kept for its attribute
                            // cache), which needs the identical fallback.
                            let attr = attr.or_else(|| {
                                if matches!(name.as_str(), "with_traceback" | "add_note" | "__traceback__" | "__context__" | "__cause__" | "__suppress_context__" | "__notes__")
                                    && crate::object::find_exception_base_name(typ).is_some() {
                                    Some(match name.as_str() {
                                        "with_traceback" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "with_traceback".to_string(),
                                            func: |args| {
                                                if args.is_empty() { return Err(PyError::type_error("with_traceback() takes exactly one argument")); }
                                                Ok(args[0].clone())
                                            },
                                            self_obj: obj.clone(),
                                        }),
                                        "add_note" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "add_note".to_string(),
                                            func: |_args| Ok(py_none()),
                                            self_obj: obj.clone(),
                                        }),
                                        // See the matching fix (and its full
                                        // explanation) in `get_attribute_impl`'s
                                        // copy of this same list (`attrs.rs`) —
                                        // `__cause__` was missing from both.
                                        "__context__" | "__traceback__" | "__cause__" => py_none(),
                                        "__suppress_context__" => py_bool(false),
                                        "__notes__" => py_list(vec![]),
                                        _ => unreachable!(),
                                    })
                                } else {
                                    None
                                }
                            });
                            match attr {
                                Some(val) => {
                                    // Cache attribute for future accesses — but
                                    // ONLY when it was found on the TYPE's own
                                    // dict/mro (a method or class attribute,
                                    // identical for every instance of this
                                    // type), never when it came from the
                                    // INSTANCE's own dict. A plain instance
                                    // attribute (`self.v`) varies per-instance,
                                    // but this cache is keyed only by
                                    // `(name_idx, type_tag)` — with no
                                    // per-instance component at all — so
                                    // caching an instance-dict hit here meant
                                    // ANY second instance of the same type
                                    // accessed via the same attribute name
                                    // within the same frame (e.g. `self.v` vs
                                    // `other.v` inside `__lt__(self, other)`)
                                    // silently got back the FIRST instance's
                                    // value instead of its own — a severe,
                                    // general correctness bug, not merely a
                                    // missed-cache-hit inefficiency. Confirmed
                                    // via a minimal repro: `other.v` returning
                                    // `self.v`'s value inside a two-argument
                                    // comparison method.
                                    //
                                    // A `property`'s (or any other `__get__`-
                                    // based descriptor's) getter is called
                                    // ABOVE and only its RETURN VALUE reaches
                                    // this point — that value is exactly as
                                    // instance-specific as a plain instance
                                    // attribute (it's computed FROM the
                                    // instance's own state), so caching it
                                    // here under the same "found on the type"
                                    // reasoning is the identical bug in a
                                    // different disguise: every instance of
                                    // the class sharing this one cache slot
                                    // got back the FIRST instance's computed
                                    // value forever after. Confirmed via a
                                    // minimal, `__slots__`-free repro: `class
                                    // Foo: x = property(lambda self: self.v)`
                                    // — `b.x` returned `a.x`'s value.
                                    let is_property_result = {
                                        let typ_ref = typ.borrow();
                                        if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                                            let found_val: Option<PyObjectRef> = type_dict.get_str(&name).cloned().or_else(|| {
                                                mro.iter().skip(1).find_map(|base| {
                                                    if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                                        base_dict.get_str(&name).cloned()
                                                    } else { None }
                                                })
                                            });
                                            found_val.map(|v| matches!(&*v.borrow(), PyObject::Property(_))).unwrap_or(false)
                                        } else {
                                            false
                                        }
                                    };
                                    // A method bound to THIS instance's own
                                    // native backing (deque subclass: `pop`/
                                    // `append`/... resolved via the native
                                    // delegation at `get_attribute_impl`) is
                                    // inherently per-instance — caching it
                                    // under a `(name_idx, type_tag)` key with
                                    // no per-instance component means the NEXT
                                    // instance of the same class in this frame
                                    // silently reuses a method still bound to
                                    // the FIRST instance's backing and mutates
                                    // the wrong object (confirmed via a deque
                                    // subclass's `d.pop(); e.pop()` in one
                                    // frame). `PyObject::Closure` values are
                                    // excluded for the same reason: a
                                    // per-instance closure (e.g. a deque
                                    // subclass's `__copy__`, which captures
                                    // that instance's own backing) must not
                                    // leak into a cache another instance
                                    // reuses.
                                    let is_native_backing_bound = matches!(&*val.borrow(), PyObject::BuiltinMethod { self_obj, .. }
                                        if !matches!(&*self_obj.borrow(), PyObject::Instance { .. } | PyObject::None))
                                        || matches!(&*val.borrow(), PyObject::Closure(_));
                                    if !dict.contains_key(&name) && !is_property_result && !is_native_backing_bound && name_idx < self.frames[fi].attr_cache.len() {
                                        self.frames[fi].attr_cache[name_idx] = Some((type_tag, val.clone()));
                                    }
                                    Ok(val)
                                }
                                None => {
                                    // Check for __getattr__ before erroring —
                                    // via the full mro, not just typ's own
                                    // dict: `__getattr__` is very commonly
                                    // defined on a BASE class (e.g. Django's
                                    // `LazyObject.__getattr__ =
                                    // new_method_proxy(getattr)`, inherited
                                    // by `SimpleLazyObject`) rather than
                                    // redeclared on every subclass, and the
                                    // instance's own exact class rarely
                                    // defines it directly.
                                    let getattr_method = crate::object::lookup_dunder_via_mro(typ, "__getattr__");
                                    if let Some(getattr_method) = getattr_method {
                                        self.call_function(getattr_method, vec![obj.clone(), py_str(&name)], vec![])
                                    } else if name == "__doc__" {
                                        // Every real object has __doc__ (defaults to
                                        // None) — see the matching fallback in
                                        // object.rs's ObjectAccess::get_attribute.
                                        Ok(py_none())
                                    } else {
                                        Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", crate::object::get_type_name_for_instance(typ), name)))
                                    }
                                }
                            }
                        }
                        _ => {
                            let type_name = obj_borrowed.type_name();
                            // Check inline cache first
                            let cached = ATTR_CACHE.with(|c| c.borrow().get(&(type_name.clone(), name.clone())).copied());
                            if let Some(func) = cached {
                                drop(obj_borrowed);
                                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func,
                                    self_obj: obj.clone(),
                                }))
                            } else {
                                let is_type_obj = matches!(&*obj_borrowed, PyObject::Type { .. });
                                let direct = obj_borrowed.get_attribute(&name);
                                let obj_type_name_for_err = obj_borrowed.type_name();
                                let attr = match direct {
                                    Ok(v) => v,
                                    Err(_) => {
                                        // Metaclass attribute fallback: a
                                        // class-level attribute not found on
                                        // the class's own dict/mro may still
                                        // exist on its *metaclass* (e.g. a
                                        // `@property` defined on a custom
                                        // metaclass like Django's
                                        // `ChoicesType.choices` — meant to be
                                        // read as `SomeChoicesClass.choices`,
                                        // with the class itself as the
                                        // property's "self"). Ordinary
                                        // classes have no METATYPE_KEY set,
                                        // so this is a no-op for them.
                                        let metatype_hit = if is_type_obj {
                                            crate::object::metatype_of(&obj).and_then(|mt| {
                                                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                                                    for base in mro.iter() {
                                                        if let PyObject::Type { dict, .. } = &*base.borrow() {
                                                            if let Some(val) = dict.get_str(&name) {
                                                                return Some(val.clone());
                                                            }
                                                        }
                                                    }
                                                }
                                                None
                                            })
                                        } else {
                                            None
                                        };
                                        match metatype_hit {
                                            Some(val) => {
                                                let is_property = if let PyObject::Property(ref d) = &*val.borrow() { d.getter.is_some() } else { false };
                                                if is_property {
                                                    let getter = if let PyObject::Property(ref d) = &*val.borrow() { d.getter.clone().unwrap() } else { unreachable!() };
                                                    drop(obj_borrowed);
                                                    let result = self.call_function(getter, vec![obj.clone()], vec![])?;
                                                    self.frames[fi].push(result);
                                                    return Ok(None);
                                                }
                                                // `obj` (a class) is being accessed as an
                                                // INSTANCE of its own metaclass here (that's
                                                // what "found on the metatype, not on obj's
                                                // own dict/mro" means) — an ordinary method
                                                // found this way must auto-bind `self=obj`,
                                                // exactly like any instance accessing a
                                                // regular method, or its first real
                                                // parameter (e.g. Django's metaclass method
                                                // `add_to_class(cls, name, value)`, called as
                                                // `new_class.add_to_class(name, value)`) never
                                                // gets bound and every later positional arg
                                                // silently shifts left by one. This is
                                                // distinct from ordinary `SomeClass.method`
                                                // access (the `is_function => Ok(attr)` case
                                                // below), which correctly stays unbound.
                                                if matches!(&*val.borrow(), PyObject::Function(_)) {
                                                    drop(obj_borrowed);
                                                    self.frames[fi].push(PyObjectRef::imm(PyObject::BoundMethod {
                                                        func: val,
                                                        self_obj: obj.clone(),
                                                    }));
                                                    return Ok(None);
                                                }
                                                val
                                            }
                                            None => {
                                                drop(obj_borrowed);
                                                if name == "__doc__" {
                                                    self.frames[fi].push(py_none());
                                                    return Ok(None);
                                                }
                                                return Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", obj_type_name_for_err, name)));
                                            }
                                        }
                                    }
                                };
                                drop(obj_borrowed);
                                // Generic descriptor protocol for class-level
                                // attribute access (`Foo.attr`, `obj` here is
                                // the type itself): a plain user-defined
                                // descriptor class (any Instance whose type
                                // defines __get__ — e.g. Django's
                                // class_or_instance_method) must have __get__
                                // invoked with instance=None, matching the
                                // generic __get__ handling already done for
                                // instance-level access above. Builtin
                                // Property/StaticMethod/ClassMethod/Function
                                // descriptors are already special-cased below
                                // and are never PyObject::Instance, so this
                                // can't double-invoke them.
                                if is_type_obj {
                                    if matches!(&*attr.borrow(), PyObject::Instance { .. }) {
                                        if let Ok(get_fn) = attr.borrow().get_attribute("__get__") {
                                            if std::env::var("RPY_DEBUG_DESCRIPTOR2").is_ok() {
                                                eprintln!("CLASS-LEVEL __get__: attr_name={} obj_type={:?}", name, obj.borrow().type_name());
                                            }
                                            let result = self.call_function(get_fn, vec![attr.clone(), py_none(), obj.clone()], vec![])?;
                                            self.frames[fi].push(result);
                                            return Ok(None);
                                        }
                                    }
                                }
                                // Resolve classmethod descriptor for type attribute access
                                {
                                    let ab = attr.borrow();
                                    if let PyObject::ClassMethod { func } = &*ab {
                                        let func_clone = func.clone();
                                        let cls_obj = obj.clone();
                                        drop(ab);
                                        let bound = PyObjectRef::new(PyObject::BoundMethod {
                                            func: func_clone,
                                            self_obj: cls_obj,
                                        });
                                        self.frames[fi].push(bound);
                                        return Ok(None);
                                    }
                                }
                                // Only rebind self_obj (and cache the
                                // func-pointer fast path keyed on it) when
                                // the found `BuiltinMethod`'s own self_obj is
                                // still the `PyObject::None` PLACEHOLDER —
                                // the established convention native
                                // container methods (File/List/Dict/Set/
                                // frozenset's own `.append`/`.get`/etc.) use,
                                // meaning "rebind me to whatever object I
                                // was actually looked up on". A
                                // BuiltinMethod that's already bound to some
                                // OTHER real object (e.g. a MODULE-level
                                // `iskeyword = frozenset(kwlist).__contains__`
                                // — self_obj is that frozenset, permanently,
                                // and `obj` here is the *module* being
                                // accessed as `keyword.iskeyword`) must be
                                // returned completely unchanged. Previously
                                // this unconditionally rebuilt EVERY
                                // BuiltinMethod found this way with
                                // `self_obj: obj.clone()`, discarding the
                                // real target and substituting the
                                // currently-accessed object instead —
                                // confirmed general via `import keyword;
                                // keyword.iskeyword("if")` raising
                                // `RuntimeError: __contains__ on
                                // non-frozenset` (self_obj had silently
                                // become the `keyword` module itself).
                                let is_placeholder_self = matches!(&*attr.borrow(), PyObject::BuiltinMethod { self_obj, .. } if matches!(&*self_obj.borrow(), PyObject::None));
                                let is_function = matches!(&*attr.borrow(), PyObject::Function(_));
                                if is_placeholder_self {
                                    let (n, func) = {
                                        let b = attr.borrow();
                                        if let PyObject::BuiltinMethod { name: n, func, .. } = &*b {
                                            (n.clone(), *func)
                                        } else { unreachable!() }
                                    };
                                    // Cache for next time — but NOT
                                    // `__init__` (nor `__new__`): a native
                                    // VALUE's `__init__` (e.g. a raw deque's,
                                    // resolved via `attrs.rs`'s per-value arm)
                                    // and the same-name TYPE-level attribute
                                    // (`deque.__init__`, the native-base
                                    // initializer) are DIFFERENT methods, yet
                                    // this cache is keyed only by
                                    // `(type_name, name)` — caching the
                                    // value-level one made `deque.__init__`
                                    // silently return the wrong function after
                                    // any `d.__init__(...)` call.
                                    if n != "__init__" && n != "__new__" {
                                        ATTR_CACHE.with(|c| { c.borrow_mut().insert((type_name.clone(), n.clone()), func); });
                                    }
                                    Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: n,
                                        func,
                                        self_obj: obj.clone(),
                                    }))
                                } else if is_function {
                                    Ok(attr)
                                } else {
                                    Ok(attr)
                                }
                            }
                        }
                    }
                }?;
                self.frames[fi].push(result);
            }

            Opcode::STORE_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                if std::env::var("RPY_DEBUG_ATTR").is_ok() {
                    let kind = match &*obj.borrow() {
                        PyObject::Type { name: n, .. } => format!("Type({})", n),
                        PyObject::Module { name: n, .. } => format!("Module({})", n),
                        PyObject::Instance { .. } => "Instance".to_string(),
                        other => format!("{:?}", std::mem::discriminant(other)),
                    };
                    eprintln!("STORE_ATTR: name={} obj_kind={}", name, kind);
                }

                // Check for __setattr__ on Instance types first
                {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type { dict: type_dict, .. } = &*typ_ref {
                            if let Some(setattr_method) = type_dict.get_str("__setattr__").cloned() {
                                drop(typ_ref);
                                drop(obj_borrowed);
                                // Call __setattr__ for side effects (validation, clearing caches)
                                let result = self.call_function(setattr_method, vec![obj.clone(), py_str(&name), val.clone()], vec![]);
                                // Also set the attribute directly in the instance dict, since
                                // __dict__ returns a COPY and self.__dict__[key] = value inside
                                // __setattr__ would modify the copy, not the original.
                                if let PyObject::Instance { dict, .. } = &mut *obj.borrow_mut() {
                                    dict.insert_str(&name, val.clone());
                                }
                                result?;
                                return Ok(None);
                            }
                        }
                    }
                }

                // Check for __set__ descriptor protocol on Instance types
                let descriptor_clone = {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type { dict: type_dict, .. } = &*typ_ref {
                            type_dict.get_str(&name).cloned()
                        } else { None }
                    } else { None }
                };
                if let Some(descriptor) = descriptor_clone {
                    // Property is special-cased directly (matching how LOAD_ATTR's
                    // getter path already calls `self.call_function` on the real
                    // getter function directly, not through a wrapper) instead of
                    // going through the generic `get_attribute("__set__")` below.
                    // That generic path returns a `BuiltinMethod` whose closure
                    // body calls the free `call_bound_method` function internally
                    // (a plain `fn(&[PyObjectRef])` has no `&mut VirtualMachine`
                    // to call through) — which spins up a disposable VM with an
                    // empty module registry. A property setter that does a lazy
                    // `import` internally (a real, common Django pattern used
                    // specifically to sidestep circular imports) would then
                    // re-import everything from scratch in that disposable VM
                    // instead of seeing what's already loaded.
                    let property_setter = {
                        let d = descriptor.borrow();
                        if let PyObject::Property(ref data) = &*d { data.setter.clone() } else { None }
                    };
                    if let Some(setter_fn) = property_setter {
                        self.call_function(setter_fn, vec![obj.clone(), val.clone()], vec![])?;
                        return Ok(None);
                    }
                    let setter_method = {
                        descriptor.borrow().get_attribute("__set__").ok()
                    };
                    if let Some(setter_method) = setter_method {
                        let result = self.call_function(setter_method, vec![descriptor, obj.clone(), val.clone()], vec![]);
                        match result {
                            Ok(_) => return Ok(None),
                            Err(e) => return Err(e),
                        }
                    } else {
                        // Descriptor exists but no __set__ (non-data descriptor), fall through
                    }
                }
                // `obj.borrow_mut()` panics unconditionally for any
                // non-`Mut`-wrapped value (SmallInt/SmallBool/SmallFloat/
                // SmallStr/None, or an `Imm`-wrapped Tuple/Bytes/Function/
                // Type/Code/boxed-Int/Str/Float) — genuinely attribute-
                // settable things (Instance, Type, Module, Exception) are
                // ALWAYS `Mut` in this codebase, so anything reaching here
                // that ISN'T `Mut` is a real attempt to set an attribute on
                // an immutable/inline value (`(5).x = 1`, `"s".x = 1`,
                // `(1, 2).x = 1`) — real CPython raises a plain
                // `AttributeError` there, not a process-ending crash. This
                // was one of the highest-impact bugs found this session:
                // it crashed the WHOLE interpreter process (not just the
                // current statement) for something this common — including
                // every test file that deliberately checks this raises via
                // `self.assertRaises(AttributeError, setattr, x, 'attr', v)`.
                if !matches!(&obj, PyObjectRef::Mut(_)) {
                    return Err(PyError::attribute_error(format!(
                        "'{}' object has no attribute '{}'", obj.borrow().type_name(), name
                    )));
                }
                obj.borrow_mut().set_attribute(&name, val)?;
            }

            Opcode::STORE_SUBSCR => {
                let val = self.frames[fi].pop()?;
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                // If `obj` is an Instance with a Python-defined __setitem__,
                // call it via `self.call_function` (the real, already-live
                // VM) rather than falling into the free `py_setitem`
                // function's own Instance-dispatch, which calls it via
                // `call_bound_method` — a separate, pre-existing, documented
                // limitation that spins up a brand-new disposable
                // `VirtualMachine::new()` for the call. That's merely
                // wasteful for most code, but genuinely catastrophic for
                // any dict-subclass with a custom `__setitem__` used during
                // this VM's own construction (e.g. enum's `_EnumDict`,
                // whose `EnumType.__new__` does `namespace[key] = ...`) —
                // the disposable VM's construction re-runs the same
                // stdlib bootstrap, hits the same assignment again, and
                // recurses without end (confirmed via gdb backtrace).
                // Falls back to the free function for everything else
                // (native list/dict/tuple assignment, or an Instance with
                // no override delegating to its native backing), which
                // needs no VM access at all.
                let setitem_fn = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__setitem__")
                } else {
                    None
                };
                if let Some(f) = setitem_fn {
                    self.call_function(f, vec![obj.clone(), index, val], vec![])?;
                } else {
                    py_setitem(&obj, &index, val)?;
                }
            }

            Opcode::DELETE_SUBSCR => {
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                py_delitem(&obj, &index)?;
            }

            Opcode::DELETE_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let obj = self.frames[fi].pop()?;
                // Check for __delattr__ on Instance types first
                {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type { dict: type_dict, .. } = &*typ_ref {
                            if let Some(delattr_method) = type_dict.get_str("__delattr__").cloned() {
                                drop(typ_ref);
                                drop(obj_borrowed);
                                self.call_function(delattr_method, vec![obj.clone(), py_str(&name)], vec![])?;
                                return Ok(None);
                            }
                        }
                    }
                }
                // Check for __delete__ descriptor protocol
                let descriptor = {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type { dict: type_dict, .. } = &*typ_ref {
                            type_dict.get_str(&name).cloned()
                        } else { None }
                    } else { None }
                };
                if let Some(ref desc) = descriptor {
                    if let Ok(deleter) = desc.borrow().get_attribute("__delete__") {
                        let result = self.call_function(deleter, vec![desc.clone(), obj.clone()], vec![]);
                        match result {
                            Ok(_) => return Ok(None),
                            Err(e) => return Err(e),
                        }
                    }
                }
                // `.borrow_mut()` panics unconditionally for anything that
                // ISN'T `PyObjectRef::Mut` — every inline variant plus any
                // `Imm`-wrapped value (boxed Int, Range, Tuple, Str, ...).
                // `del some_immutable_value.attr` (real trigger: CPython's
                // own `test_range.py`, `del rangeobj.start` — a `range`
                // object's `start`/`stop`/`step` are read-only, expected to
                // raise a clean `AttributeError`) previously panicked the
                // whole process instead. Same fix shape as `builtin_setattr`
                // already applies for `setattr()`.
                if !matches!(obj, PyObjectRef::Mut(_)) {
                    return Err(PyError::attribute_error(format!(
                        "'{}' object attribute '{}' is read-only", obj.borrow().type_name(), name
                    )));
                }
                obj.borrow_mut().del_attribute(&name)?;
            }

            Opcode::LIST_APPEND => {
                let val = self.frames[fi].pop()?;
                let list = self.frames[fi].peek(arg as usize)?;
                let mut obj = list.borrow_mut();
                if let PyObject::List(v) = &mut *obj {
                    v.push(val);
                } else {
                    return Err(PyError::runtime_error("LIST_APPEND on non-list"));
                }
            }

            Opcode::LIST_EXTEND => {
                let val = self.frames[fi].pop()?;
                let items: Vec<PyObjectRef> = {
                    let val_ref = val.borrow();
                    match &*val_ref {
                        PyObject::List(v) => v.clone(),
                        PyObject::Tuple(v) => v.clone(),
                        _ => {
                            drop(val_ref);
                            let iterator = crate::object::builtin_iter(&[val.clone()])
                                .map_err(|_| PyError::runtime_error("LIST_EXTEND requires an iterable"))?;
                            let mut result = Vec::new();
                            loop {
                                match crate::object::builtin_next(&[iterator.clone()]) {
                                    Ok(item) => result.push(item),
                                    Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            result
                        }
                    }
                };
                let list = self.frames[fi].peek(arg as usize)?;
                let mut obj = list.borrow_mut();
                if let PyObject::List(v) = &mut *obj {
                    v.extend(items);
                } else {
                    return Err(PyError::runtime_error("LIST_EXTEND on non-list"));
                }
            }

            Opcode::SET_ADD => {
                let val = self.frames[fi].pop()?;
                let set = self.frames[fi].peek(arg as usize)?;
                let mut obj = set.borrow_mut();
                if let PyObject::Set(v) = &mut *obj {
                    v.add(val)?;
                } else {
                    return Err(PyError::runtime_error("SET_ADD on non-set"));
                }
            }

            Opcode::SET_UPDATE => {
                // Backs `{*a, *b}` set-unpacking display syntax. Unlike
                // LIST_EXTEND (which only accepts list/tuple), CPython's
                // SET_UPDATE accepts any iterable, so pull items through
                // the same builtin_iter/builtin_next protocol chain() uses
                // rather than requiring a concrete List/Tuple/Set.
                let val = self.frames[fi].pop()?;
                let mut items = Vec::new();
                let it = crate::object::builtin_iter(&[val])?;
                loop {
                    match crate::object::builtin_next(&[it.clone()]) {
                        Ok(v) => items.push(v),
                        Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                        Err(e) => return Err(e),
                    }
                }
                let set = self.frames[fi].peek(arg as usize)?;
                let mut obj = set.borrow_mut();
                if let PyObject::Set(v) = &mut *obj {
                    for item in items {
                        v.add(item)?;
                    }
                } else {
                    return Err(PyError::runtime_error("SET_UPDATE on non-set"));
                }
            }

            Opcode::MAP_ADD => {
                let val = self.frames[fi].pop()?;
                let key = self.frames[fi].pop()?;
                let map = self.frames[fi].peek(arg as usize)?;
                let mut obj = map.borrow_mut();
                if let PyObject::Dict(d) = &mut *obj {
                    d.set(key, val)?;
                } else {
                    return Err(PyError::runtime_error("MAP_ADD on non-dict"));
                }
            }

            Opcode::DICT_MERGE => {
                let source = self.frames[fi].pop()?;
                let target = self.frames[fi].peek(arg as usize)?;
                let source_items = {
                    let src_borrowed = source.borrow();
                    match &*src_borrowed {
                        PyObject::Dict(d) => d.items(),
                        _ => return Err(PyError::type_error(
                            format!("cannot merge non-dict into dict"))),
                    }
                };
                let mut target_borrowed = target.borrow_mut();
                if let PyObject::Dict(td) = &mut *target_borrowed {
                    for (k, v) in source_items {
                        td.set(k, v)?;
                    }
                } else {
                    return Err(PyError::runtime_error("DICT_MERGE on non-dict"));
                }
            }

            Opcode::LIST_TO_TUPLE => {
                let list = self.frames[fi].pop()?;
                let items = match &*list.borrow() {
                    PyObject::List(v) => v.clone(),
                    _ => return Err(PyError::runtime_error("LIST_TO_TUPLE on non-list")),
                };
                self.frames[fi].push(PyObjectRef::imm(PyObject::Tuple(items)));
            }

            Opcode::UNPACK_SEQUENCE => {
                let count = arg as usize;
                let seq = self.frames[fi].pop()?;
                let list_items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => Some(v.clone()),
                        _ => None,
                    }
                };
                // Any other iterable (generator, custom __iter__, set, dict
                // view, str, ...) — real Python's unpacking assignment
                // accepts anything iterable, not just list/tuple literally.
                // Confirmed missing via Django's real `for k, v in
                // some_dict.items():`-adjacent unpacking during
                // `django.setup()`.
                let items = match list_items {
                    Some(v) => v,
                    None => {
                        let iterator = crate::object::builtin_iter(&[seq.clone()])
                            .map_err(|_| PyError::type_error(format!("cannot unpack non-iterable '{}' object", seq.borrow().type_name())))?;
                        let mut v = Vec::new();
                        loop {
                            match crate::object::builtin_next(&[iterator.clone()]) {
                                Ok(val) => v.push(val),
                                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                Err(e) => return Err(e),
                            }
                        }
                        v
                    }
                };
                if items.len() != count {
                    // Match real CPython's exact wording (confirmed against
                    // a real interpreter) — the previous generic "cannot
                    // unpack N items into M values" message matched neither
                    // phrasing CPython actually uses.
                    return Err(PyError::value_error(if items.len() < count {
                        format!("not enough values to unpack (expected {}, got {})", count, items.len())
                    } else {
                        format!("too many values to unpack (expected {})", count)
                    }));
                }
                for item in items.into_iter().rev() {
                    self.frames[fi].push(item);
                }
            }

            Opcode::UNPACK_EX => {
                let before = (arg >> 8) as usize;
                let after = (arg & 0xFF) as usize;
                let total = before + after + 1; // +1 for the starred item
                let seq = self.frames[fi].pop()?;
                let list_items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => Some(v.clone()),
                        _ => None,
                    }
                };
                // Same generalization as UNPACK_SEQUENCE above: fall back to
                // the real iterator protocol for anything that isn't
                // literally a list/tuple.
                let items = match list_items {
                    Some(v) => v,
                    None => {
                        let iterator = crate::object::builtin_iter(&[seq.clone()])
                            .map_err(|_| PyError::type_error(format!("cannot unpack non-iterable '{}' object", seq.borrow().type_name())))?;
                        let mut v = Vec::new();
                        loop {
                            match crate::object::builtin_next(&[iterator.clone()]) {
                                Ok(val) => v.push(val),
                                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                Err(e) => return Err(e),
                            }
                        }
                        v
                    }
                };
                if items.len() < before + after {
                    // Matches real CPython's wording for starred unpacking
                    // (`a, *b, c = seq`) — "at least" since a starred target
                    // can absorb any number of extra items above the
                    // minimum (`before + after`), unlike plain
                    // `UNPACK_SEQUENCE`'s exact-count requirement.
                    return Err(PyError::value_error(format!(
                        "not enough values to unpack (expected at least {}, got {})", before + after, items.len()
                    )));
                }
                let _ = total;
                let n = items.len();
                // Push order (bottom of stack = first to be stored):
                //   before items, star list, after items
                // So we push in reverse: after items first (on top), then star list, then before items
                // Push after-star items (last N) in reverse
                for i in (n - after)..n {
                    self.frames[fi].push(items[i].clone());
                }
                // Push starred item (everything between before and after) as a list
                let star_count = n - before - after;
                let mut star_items: Vec<PyObjectRef> = Vec::new();
                for i in before..(before + star_count) {
                    star_items.push(items[i].clone());
                }
                self.frames[fi].push(py_list(star_items));
                // Push before-star items (first N) in reverse so first comes out on bottom
                for i in (0..before).rev() {
                    self.frames[fi].push(items[i].clone());
                }
            }

            Opcode::SETUP_FINALLY => {
                let stack_depth = self.frames[fi].stack.len();
                let handler = ExceptionHandler {
                    instr_addr: arg as usize,
                    stack_depth,
                };
                self.frames[fi].exception_handlers.push(handler);
            }

            Opcode::SETUP_CLEANUP => {
                let stack_depth = self.frames[fi].stack.len();
                let handler = ExceptionHandler {
                    instr_addr: arg as usize,
                    stack_depth,
                };
                self.frames[fi].exception_handlers.push(handler);
            }

            Opcode::POP_BLOCK => {
                // Restore stack to the depth before the handler was set up
                if let Some(handler) = self.frames[fi].exception_handlers.pop() {
                    self.frames[fi].stack.truncate(handler.stack_depth);
                }
            }

            Opcode::PUSH_EXC_INFO => {
                // Save TOS to active_exception without popping (the exception
                // stays on the value stack for DUP_TOP/CHECK_EXC_MATCH below).
                // This provides a stable source for RERAISE even after POP_EXCEPT
                // pops the exception from the value stack (as in try/finally).
                if let Ok(exc) = self.frames[fi].peek(0) {
                    self.frames[fi].active_exception = Some(Box::new(exc));
                }
            }

            Opcode::POP_EXCEPT => {
                // Pop the exception object from the value stack.
                // In CPython this operates on a separate block stack for
                // exception info (type, value, traceback). Since RustPython
                // places the exception directly on the value stack, we pop
                // it here. The exception may already have been consumed by
                // STORE_NAME/STORE_FAST (handler with 'as e'), or it may
                // still be on the stack (handler without 'as e').
                self.frames[fi].stack.pop();
            }

            Opcode::GET_AITER => {
                // async for: call __aiter__ on the top of stack
                let obj = self.frames[fi].peek(0)?;
                let aiter_method = obj.borrow().get_attribute("__aiter__")
                    .map_err(|_| PyError::type_error("object does not support async iteration"))?;
                let result = self.call_function(aiter_method, vec![], vec![])?;
                let _ = self.frames[fi].pop();
                self.frames[fi].push(result);
            }

            Opcode::GET_ANEXT => {
                // async for: get __anext__ method from the async iterator
                let obj = self.frames[fi].peek(0)?;
                let anext_method = obj.borrow().get_attribute("__anext__")
                    .map_err(|_| PyError::type_error("async iterator has no __anext__"))?;
                let _ = self.frames[fi].pop();
                self.frames[fi].push(anext_method);
            }

            Opcode::END_FOR => {
                // Pop the iterator/async-iterator after a for loop
                let _ = self.frames[fi].pop();
            }

            Opcode::BEFORE_ASYNC_WITH => {
                // async with: call __aenter__ and push __aexit__ for later
                let mgr = self.frames[fi].pop()?;
                let aenter_method = mgr.borrow().get_attribute("__aenter__")
                    .map_err(|_| PyError::attribute_error("async context manager has no __aenter__"))?;
                let result = self.call_function(aenter_method, vec![], vec![])?;
                self.frames[fi].push(mgr);
                self.frames[fi].push(result);
            }

            Opcode::CHECK_EXC_MATCH => {
                let expected = self.frames[fi].pop()?;
                let exc = self.frames[fi].pop()?;
                let is_instance = matches!(&*exc.borrow(), PyObject::Instance { .. });
                let matched = if is_instance {
                    // A user-defined exception CLASS instance (`class
                    // MyError(Exception): ...`, `raise MyError(...)`) — used
                    // to fall straight to `None => false` below, since only
                    // the native `PyObject::Exception`/`ExceptionGroup`
                    // representations were recognized at all. This meant
                    // `except AnythingAtAll:` NEVER matched ANY user-defined
                    // exception class, no matter what it inherited from —
                    // confirmed via the simplest possible repro (`class
                    // MyError(Exception): pass; raise MyError("x")` not even
                    // caught by `except MyError:`, an EXACT match). Delegate
                    // to `isinstance()`, which is the real semantic `except`
                    // matching means, and which already correctly walks both
                    // custom-class mro (exact/ancestor custom matches) and
                    // `find_exception_base_name` (matches against whatever
                    // real builtin exception the class ultimately derives
                    // from, e.g. `except AttributeError:` catching a
                    // `class Foo(AttributeError): ...`).
                    crate::object::builtin_isinstance(&[exc.clone(), expected.clone()])?.truthy()
                } else {
                    let typ_name = match &*exc.borrow() {
                        PyObject::Exception { typ, .. } => Some(typ.clone()),
                        PyObject::ExceptionGroup { typ, .. } => Some(typ.clone()),
                        _ => None,
                    };
                    match typ_name {
                        Some(t) => exc_type_matches(&expected, &t)?,
                        None => false,
                    }
                };
                self.frames[fi].push(py_bool(matched));
            }

            Opcode::CHECK_EXC_MATCH_STAR => {
                // For except*: splits ExceptionGroup into matched/unmatched subgroups.
                // Pops 3 items (type, exc_dup from DUP_TOP, exc_orig from before DUP_TOP).
                // On match: pushes [unmatched_eg_or_empty_eg, matched_eg, True].
                // On no match: pushes [exc_orig, False].
                let expected = self.frames[fi].pop()?;
                let exc_dup = self.frames[fi].pop()?;
                let exc_orig = self.frames[fi].pop()?;

                // Read the type info from exc_dup while we still hold the borrow
                let is_eg = match &*exc_dup.borrow() {
                    PyObject::ExceptionGroup { .. } => true,
                    _ => false,
                };

                if is_eg {
                    // Read fully from the borrow so we can drop it
                    let (typ, args, matched, unmatched) = {
                        let eg = &*exc_dup.borrow();
                        let (typ, args, exceptions) = match eg {
                            PyObject::ExceptionGroup { typ, args, exceptions } => (typ.clone(), args.clone(), exceptions.clone()),
                            _ => unreachable!(),
                        };
                        let mut matched = Vec::new();
                        let mut unmatched = Vec::new();
                        for child in &exceptions {
                            let child_name = match &*child.borrow() {
                                PyObject::Exception { typ, .. } => typ.clone(),
                                PyObject::ExceptionGroup { typ, .. } => typ.clone(),
                                _ => String::new(),
                            };
                            if exc_type_matches(&expected, &child_name)? {
                                matched.push(child.clone());
                            } else {
                                unmatched.push(child.clone());
                            }
                        }
                        (typ, args, matched, unmatched)
                    };

                    if !matched.is_empty() {
                        let matched_group = PyObjectRef::new(PyObject::ExceptionGroup {
                            typ: typ.clone(),
                            args: args.clone(),
                            exceptions: matched,
                        });
                        if !unmatched.is_empty() {
                            let unmatched_group = PyObjectRef::new(PyObject::ExceptionGroup {
                                typ: typ.clone(),
                                args: vec![py_str(&typ)],
                                exceptions: unmatched,
                            });
                            self.frames[fi].push(unmatched_group);
                        } else {
                            let empty_group = PyObjectRef::new(PyObject::ExceptionGroup {
                                typ: typ.clone(),
                                args: vec![py_str(&typ)],
                                exceptions: vec![],
                            });
                            self.frames[fi].push(empty_group);
                        }
                        self.frames[fi].push(matched_group);
                        self.frames[fi].push(py_bool(true));
                    } else {
                        // No matching children: restore original exception
                        self.frames[fi].push(exc_orig);
                        self.frames[fi].push(py_bool(false));
                    }
                } else {
                    // Not an ExceptionGroup — normal match check
                    let typ_name = match &*exc_dup.borrow() {
                        PyObject::Exception { typ, .. } => Some(typ.clone()),
                        _ => None,
                    };
                    let matched = match typ_name {
                        Some(t) => exc_type_matches(&expected, &t)?,
                        None => false,
                    };
                    if matched {
                        let empty_group = PyObjectRef::new(PyObject::ExceptionGroup {
                            typ: "ExceptionGroup".to_string(),
                            args: vec![py_str("")],
                            exceptions: vec![],
                        });
                        self.frames[fi].push(empty_group);
                        self.frames[fi].push(exc_dup);
                        self.frames[fi].push(py_bool(true));
                    } else {
                        self.frames[fi].push(exc_orig);
                        self.frames[fi].push(py_bool(false));
                    }
                }
            }

            Opcode::RERAISE => {
                // Prefer active_exception (set by PUSH_EXC_INFO) so that
                // POP_EXCEPT (which pops from the value stack) does not break
                // RERAISE in try/finally blocks.
                let reraise_exc = if let Some(exc) = self.frames[fi].active_exception.take() {
                    *exc
                } else {
                    match self.frames[fi].pop() {
                        Ok(exc) => exc,
                        Err(_) => {
                            if std::env::var("RPY_DEBUG_RERAISE").is_ok() {
                                eprintln!("RERAISE FAIL: func={} file={} stack_len={}",
                                    self.frames[fi].code.name, self.frames[fi].code.filename, self.frames[fi].stack.len());
                            }
                            return Err(PyError::runtime_error("No active exception to re-raise"));
                        }
                    }
                };
                // Check if it's an empty ExceptionGroup (all exceptions were handled)
                let is_empty_eg = match &*reraise_exc.borrow() {
                    PyObject::ExceptionGroup { exceptions, .. } => exceptions.is_empty(),
                    _ => false,
                };
                if !is_empty_eg {
                    if std::env::var("RPY_DEBUG_RERAISE").is_ok() {
                        eprintln!("RERAISE: kind={:?} repr={}", std::mem::discriminant(&*reraise_exc.borrow()), reraise_exc.borrow().repr());
                    }
                    return Err(PyError::Exception("re-raise".to_string(), reraise_exc));
                }
                // Empty group — all exceptions handled, silently continue
            }

            Opcode::RAISE_VARARGS => {
                let nargs = arg;
                match nargs {
                    0 => {
                        // Bare raise: re-raise the current exception. Must
                        // check `active_exception` FIRST, matching RERAISE
                        // just above — an `except E as exc:` clause's `as
                        // exc` binding (STORE_FAST) already consumes the
                        // value-stack copy of the exception, so a bare
                        // `raise` later in that same handler (the standard
                        // `except BaseException as exc: if exc is not
                        // value: raise` idiom — real code, CPython's own
                        // `contextlib._GeneratorContextManager.__exit__`)
                        // found nothing on the stack and incorrectly failed
                        // with "No active exception to re-raise" even
                        // though the real exception was still available via
                        // `active_exception` (set by PUSH_EXC_INFO exactly
                        // for this purpose, per its own doc comment).
                        let reraise_exc = if let Some(exc) = self.frames[fi].active_exception.take() {
                            Some(*exc)
                        } else {
                            self.frames[fi].stack.pop()
                        };
                        match reraise_exc {
                            Some(exc) => {
                                return Err(PyError::Exception(format!("re-raise"), exc));
                            }
                            None => return Err(PyError::runtime_error("No active exception to re-raise")),
                        }
                    }
                    1 => {
                        let exc = self.frames[fi].pop()?;
                        // If the raised value is already a native exception
                        // representation, use it directly — `PyObject::Str`
                        // deliberately NOT included here (see below): a bare
                        // string is never a valid thing to raise in Python 3.
                        let is_callable = !matches!(&*exc.borrow(),
                            PyObject::Exception { .. } | PyObject::ExceptionGroup { .. } | PyObject::Instance { .. }
                        );
                        let exc = if is_callable {
                            let exc_clone = exc.clone();
                            match self.call_function(exc_clone, vec![], vec![]) {
                                Ok(instance) => instance,
                                Err(_) => return Err(PyError::type_error("exceptions must derive from BaseException")),
                            }
                        } else {
                            exc
                        };
                        // Verify the raised value genuinely derives from
                        // BaseException. Real Python allows raising any
                        // BaseException instance/class, but a plain
                        // `class Foo: pass` instance (or a bare string —
                        // caught by excluding `PyObject::Str` from
                        // `is_callable` above, which routes it through
                        // `call_function` on a non-callable and fails there)
                        // must raise `TypeError`, not be silently treated as
                        // a legitimate, uncatchable-by-`except TypeError`
                        // exception. Real trigger: CPython's own
                        // `test_baseexception.py::
                        // test_raise_new_style_non_exception`/
                        // `test_raise_string` — both `raise SomeInstance`
                        // (no Exception ancestor) and `raise "spam"`
                        // previously propagated the raw value uncaught by
                        // ANY `except` clause instead of raising `TypeError`.
                        // `PyObject::Exception`/`ExceptionGroup` are this
                        // interpreter's own native representations, always
                        // trusted; only a plain `PyObject::Instance` needs
                        // its class hierarchy actually checked here.
                        if let PyObject::Instance { typ, .. } = &*exc.borrow() {
                            if crate::object::find_exception_base_name(typ).is_none() {
                                return Err(PyError::type_error("exceptions must derive from BaseException"));
                            }
                        }
                        let msg = match &*exc.borrow() {
                            PyObject::Str(s) => s.to_string(),
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() { args[0].str() } else { "".to_string() }
                            }
                            PyObject::ExceptionGroup { args, .. } => {
                                if !args.is_empty() { args[0].str() } else { "".to_string() }
                            }
                            PyObject::Instance { dict, .. } => {
                                // Extract error message from the instance
                                // Python stores exception args in self.args tuple
                                let args = dict.get_str("args");
                                if let Some(a) = args {
                                    let b = a.borrow();
                                    if let PyObject::Tuple(t) = &*b {
                                        if !t.is_empty() { t[0].str() }
                                        else { exc.repr() }
                                    } else { exc.repr() }
                                } else {
                                    // Fallback: repr of the exception object
                                    exc.repr()
                                }
                            }
                            _ => return Err(PyError::type_error("exceptions must be str or Exception instances")),
                        };
                        // raise StopIteration → PyError::StopIteration (needed by for_iter/next protocol)
                        if msg.is_empty() {
                            let exc_borrowed = exc.borrow();
                            let is_stop = match &*exc_borrowed {
                                PyObject::Exception { ref typ, .. } if typ == "StopIteration" => true,
                                PyObject::Type { name, .. } if name == "StopIteration" => true,
                                _ => false,
                            };
                            if is_stop {
                                return Err(PyError::StopIteration);
                            }
                        }
                        // Store exc_info before returning error. `exc_type`
                        // must be the exception's real CLASS object (what
                        // real `sys.exc_info()[0]` is, and what makes
                        // `issubclass(exc_info()[0], SomeError)` — the
                        // pattern `unittest`'s own `TestResult`/`_Outcome`
                        // use to classify failures vs. errors — valid to
                        // call at all) — NOT the exception instance itself.
                        // This used to just be `exc.clone()` (the instance)
                        // for BOTH `exc_type` and `exc_value`, so any code
                        // calling `issubclass()` on it crashed with
                        // `TypeError: issubclass() arg 1 must be a class`
                        // the moment a real test failure/error tried to get
                        // reported through `unittest.main()`.
                        self.exc_type = Some(self.exception_class_of(&exc));
                        self.exc_value = Some(exc.clone());
                        self.exc_traceback = Some(py_none());
                        if std::env::var("RPY_DEBUG_EXCINFO").is_ok() {
                            eprintln!("RAISE set exc_type={} exc_value={}", self.exc_type.as_ref().unwrap().repr(), self.exc_value.as_ref().unwrap().repr());
                        }
                        return Err(PyError::Exception(msg, exc));
                    }
                    2 => {
                        let cause = self.frames[fi].pop()?;
                        let exc = self.frames[fi].pop()?;
                        let exc_msg = match &*exc.borrow() {
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() { args[0].str() } else { exc.str() }
                            }
                            _ => exc.str(),
                        };
                        let cause_msg = match &*cause.borrow() {
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() { args[0].str() } else { cause.str() }
                            }
                            _ => cause.str(),
                        };
                        // Set __cause__ on the exception object. The native
                        // `PyObject::Exception` representation has a
                        // dedicated `cause` field; a user-defined exception
                        // class (`class MyError(Exception): ...` — a plain
                        // `PyObject::Instance`, the overwhelming majority of
                        // real exception classes) has no such field, so
                        // `raise X from Y` previously did NOTHING for it —
                        // `X.__cause__` stayed `None` regardless, silently
                        // dropping the explicit cause entirely. `Instance`'s
                        // own `set_attribute` already supports arbitrary
                        // attributes with no restriction, so just storing
                        // `__cause__` there directly works, matching how
                        // reading it back (`get_attribute_impl`'s Instance
                        // arm) already checks the instance's own dict first.
                        match &mut *exc.borrow_mut() {
                            PyObject::Exception { cause: ref mut cause_field, .. } => {
                                *cause_field = Some(cause.clone());
                            }
                            PyObject::Instance { dict, .. } => {
                                dict.insert_str("__cause__", cause.clone());
                            }
                            _ => {}
                        }
                        return Err(PyError::Exception(format!("{} (caused by: {})", exc_msg, cause_msg), exc));
                    }
                    _ => return Err(PyError::runtime_error("invalid RAISE_VARARGS count")),
                }
            }

            Opcode::IMPORT_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                // Pop level (int, TOS) and fromlist (TOS1)
                let level_val = self.frames[fi].pop()?;
                let _fromlist = self.frames[fi].pop()?;
                // Resolve relative imports: if level > 0, use __package__ from frame globals
                let resolved = {
                    let level = {
                        let obj = level_val.borrow();
                        match &*obj {
                            PyObject::Int(i) => i.to_i64().unwrap_or(0) as usize,
                            _ => 0,
                        }
                    };
                    if level > 0 {
                        let pkg = self.frames[fi].globals.borrow()
                            .get(&interner::intern("__package__")).cloned()
                            .and_then(|p| {
                                let p = p.borrow();
                                if let PyObject::Str(s) = &*p { Some(s.to_string()) } else { None }
                            });
                        // level=1 (`from . import x`) resolves relative to
                        // __package__ itself; each additional dot
                        // (level=2 → `from .. import x`, etc.) goes up one
                        // more enclosing package, stripping one more
                        // trailing component. This was previously ignored
                        // entirely — `from .. import x` resolved identically
                        // to `from . import x`, silently importing from the
                        // wrong (child, not parent) package whenever a
                        // module used a multi-dot relative import.
                        let pkg = pkg.map(|p| {
                            let mut segs: Vec<&str> = p.split('.').collect();
                            let strip = level.saturating_sub(1);
                            if strip >= segs.len() { segs.clear(); } else { segs.truncate(segs.len() - strip); }
                            segs.join(".")
                        });
                        let resolved_name = match pkg {
                            Some(p) if !p.is_empty() => {
                                if name.is_empty() { p } else { format!("{}.{}", p, name) }
                            }
                            // Fallback: use __name__ up to last dot as package
                            _ => {
                                let n = self.frames[fi].globals.borrow()
                                    .get(&interner::intern("__name__")).cloned()
                                    .and_then(|n| {
                                        let n = n.borrow();
                                        if let PyObject::Str(s) = &*n { Some(s.to_string()) } else { None }
                                    }).unwrap_or_default();
                                if let Some(dot) = n.rfind('.') {
                                    let base = &n[..dot];
                                    if name.is_empty() { base.to_string() } else { format!("{}.{}", base, name) }
                                } else { name.clone() }
                            }
                        };
                        resolved_name
                    } else {
                        name.clone()
                    }
                };
                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                    eprintln!("IMPORT_NAME: resolved={} cached={}", resolved, self.modules.contains_key(&resolved));
                }
                if let Some(module) = self.import_cached_or_fresh(&resolved) {
                    // For 'import a.b.c' where fromlist is empty (regular import, not 'from a.b import X'),
                    // push the top-level module so STORE_NAME stores the package, not the submodule
                    let is_from_import = {
                        let obj = _fromlist.borrow();
                        matches!(&*obj, PyObject::Tuple(items) if !items.is_empty())
                    };
                    if resolved.contains('.') && !is_from_import {
                        // Set sub-module as attribute on parent module (e.g. logging.config = <module>)
                        if let Some(dot_pos) = resolved.rfind('.') {
                            let parent_name = &resolved[..dot_pos];
                            let child_name = &resolved[dot_pos+1..];
                            if let Some(parent_mod) = self.modules.get(parent_name) {
                                let _ = parent_mod.borrow_mut().set_attribute(child_name, module.clone());
                            }
                        }
                        if let Some(top) = resolved.split('.').next() {
                            if let Some(top_mod) = self.modules.get(top) {
                                self.frames[fi].push(top_mod.clone());
                            } else {
                                self.frames[fi].push(module.clone());
                            }
                        } else {
                            self.frames[fi].push(module.clone());
                        }
                    } else {
                        self.frames[fi].push(module.clone());
                    }
                } else {
                    match self.import_module_from_file(&resolved) {
                        Ok(module) => {
                            self.modules.insert(resolved.clone(), module.clone());
                            // Register in sys.modules (safe: module fully loaded)
                            if let Some(sys_mod) = self.modules.get("sys") {
                                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                                    if let Some(md) = dict.get_str("modules").cloned() {
                                        match &md {
                                            PyObjectRef::Mut(rc) => {
                                                if let Ok(mut guard) = rc.try_borrow_mut() {
                                                    if let PyObject::Dict(ref mut d) = &mut *guard {
                                                        d.set(py_str(&resolved), module.clone()).ok();
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            self.frames[fi].push(module);
                            // For 'import a.b.c' where fromlist is empty,
                            // push top-level module instead of deepest module
                            let is_from_import = {
                                let obj = _fromlist.borrow();
                                matches!(&*obj, PyObject::Tuple(items) if !items.is_empty())
                            };
                            if resolved.contains('.') && !is_from_import {
                                if let Some(top) = resolved.split('.').next() {
                                    if let Some(top_mod) = self.modules.get(top) {
                                        let _ = self.frames[fi].pop();
                                        self.frames[fi].push(top_mod.clone());
                                    }
                                }
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }

            Opcode::IMPORT_FROM => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let module = self.frames[fi].peek(0)?;
                // Handle 'from module import *' — when the imported name is '*',
                // iterate over the module's dict and store all names in current scope
                if name == "*" {
                    let module_borrowed = module.borrow();
                    if let PyObject::Module { dict, .. } = &*module_borrowed {
                        // Use __all__ if present, otherwise all non-underscore names
                        let names_to_import: Vec<String> = if let Some(all_val) = dict.get_str("__all__") {
                            let all_borrowed = all_val.borrow();
                            match &*all_borrowed {
                                PyObject::Tuple(items) | PyObject::List(items) => {
                                    items.iter().filter_map(|n| {
                                        if let PyObject::Str(s) = &*n.borrow() { Some(s.to_string()) } else { None }
                                    }).collect()
                                }
                                _ => dict.keys().map(|k| interner::lookup_str(*k)).filter(|k| !k.starts_with('_')).map(|k| k.to_string()).collect(),
                            }
                        } else {
                            dict.keys().map(|k| interner::lookup_str(*k)).filter(|k| !k.starts_with('_')).map(|k| k.to_string()).collect()
                        };
                        // Collect name-value pairs before dropping borrow
                        let imports: Vec<(String, PyObjectRef)> = names_to_import.iter()
                            .filter_map(|name| dict.get_str(&name).map(|val| (name.clone(), val.clone())))
                            .collect();
                        drop(module_borrowed);
                        let live_module = self.frames[fi].live_module.clone();
                        for (import_name, val) in &imports {
                            if let Some(order) = self.frames[fi].name_order.clone() {
                                let mut order = order.borrow_mut();
                                if !order.contains(import_name) {
                                    order.push(import_name.clone());
                                }
                            }
                            if let Some(lm) = &live_module {
                                if let PyObject::Module { dict, .. } = &mut *lm.borrow_mut() {
                                    dict.insert_str(import_name, val.clone());
                                }
                            }
                            self.frames[fi].globals.borrow_mut().insert(interner::intern(&import_name), val.clone());
                        }
                        // Push placeholder module result (the loop above already pushed values)
                        // The POP_TOP after IMPORT_FROM loop will clean up
                        self.frames[fi].push(py_none());
                        return Ok(None);
                    }
                }
                // Check if name is in module's dict first (without holding borrow)
                let found = {
                    let obj = module.borrow();
                    match &*obj {
                        PyObject::Module { dict, .. } => dict.get_str(&name).cloned(),
                        _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
                    }
                };
                // Get module name for submodule import (clone to avoid borrow conflicts)
                let module_name = {
                    let obj = module.borrow();
                    match &*obj {
                        PyObject::Module { name: mn, .. } => mn.clone(),
                        _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
                    }
                };
                // Circular-import fallback: if this module is STILL mid-execution
                // further down the call stack (e.g. its __init__.py does
                // `import package.submodule` as its last statement, and that
                // submodule does `from . import name_defined_earlier`), the
                // module object's own dict is only populated once the whole
                // body finishes — it's a snapshot copy, not a live view of the
                // executing frame's globals. Check ancestor frames' actual
                // live globals for the name before giving up.
                let found_direct = found.is_some();
                let found = found.or_else(|| {
                    self.frames.iter().find_map(|f| {
                        let g = f.globals.borrow();
                        if g.get(&interner::intern("__name__")).map(|n| n.str()).as_deref() == Some(module_name.as_str()) {
                            g.get(&interner::intern(&name)).cloned()
                        } else {
                            None
                        }
                    })
                });
                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                    eprintln!("IMPORT_FROM: name={} module={} found_direct={} found_after_ancestor={}", name, module_name, found_direct, found.is_some());
                }
                if let Some(val) = found {
                    self.frames[fi].push(val);
                } else {
                    // Try importing as sub-module (for dotted names like os.path)
                    let submodule_name = format!("{}.{}", module_name, name);
                    if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                        eprintln!("IMPORT_FROM fallback: submodule_name={} already_cached={}", submodule_name, self.modules.contains_key(&submodule_name));
                    }
                    if submodule_name.contains('.') {
                        match self.import_module_from_file(&submodule_name) {
                            Ok(submod) => {
                                self.modules.insert(submodule_name.clone(), submod.clone());
                                if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                                    dict.insert_str(&name, submod.clone());
                                }
                                self.frames[fi].push(submod);
                            }
                            Err(e) => {
                                // RPY_DEBUG_IMPORT=1 prints the real underlying
                                // error before it potentially gets flattened
                                // into the generic message below (kept
                                // permanently per user request, 2026-07-19).
                                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                                    eprintln!("IMPORT_FROM_FAIL: name={} module={} err={}", name, module_name, e);
                                }
                                // Only "the submodule doesn't exist" collapses
                                // to CPython's generic "cannot import name"
                                // message here. Any other error (e.g. the
                                // submodule's own body raising a real
                                // exception) must propagate as that real
                                // exception — previously this branch always
                                // discarded it in favor of the generic
                                // ImportError, so a genuine bug inside an
                                // imported submodule was indistinguishable
                                // from the name simply not existing.
                                if matches!(e, PyError::ImportError(_)) {
                                    return Err(PyError::ImportError(format!("cannot import name '{}' from '{}'", name, module_name)));
                                }
                                return Err(e);
                            }
                        }
                    } else {
                        return Err(PyError::ImportError(format!("cannot import name '{}' from '{}'", name, module_name)));
                    }
                }
            }

            Opcode::LOAD_BUILD_CLASS => {
                self.frames[fi].push(PyObjectRef::imm(PyObject::BuildClass));
            }

            Opcode::LOAD_CLOSURE => {
                let idx = arg as usize;
                let cell = {
                    let f = &self.frames[self.frames.len() - 1];
                    if idx < f.code.cellvars.len() {
                        let name = &f.code.cellvars[idx];
                        if let Some(var_idx) = f.code.varnames.iter().position(|&n| crate::interner::intern_eq(n, name)) {
                            if let Some(val) = f.fast_locals.get(var_idx).and_then(|v| v.clone()) {
                                val
                            } else {
                                PyObjectRef::new(PyObject::Cell { value: None })
                            }
                        } else {
                            PyObjectRef::new(PyObject::Cell { value: None })
                        }
                    } else {
                        let fv_idx = idx - f.code.cellvars.len();
                        if let Some(cell) = f.closure.get(fv_idx).cloned() {
                            cell
                        } else {
                            PyObjectRef::new(PyObject::Cell { value: None })
                        }
                    }
                };
                self.frames[fi].push(cell);
            }

            Opcode::FORMAT_SIMPLE => {
                let val = self.frames[fi].pop()?;
                self.frames[fi].push(py_str(&val.str()));
            }

            Opcode::FORMAT_WITH_SPEC => {
                let spec = self.frames[fi].pop()?;
                let val = self.frames[fi].pop()?;
                let spec_str = spec.str();
                self.frames[fi].push(py_str(&format_with_spec(&val, &spec_str)?));
            }

            Opcode::CONVERT_VALUE => {
                let conversion = arg;
                let val = self.frames[fi].pop()?;
                let result = match conversion {
                    0 => py_str(&val.str()),
                    1 => py_str(&val.repr()),
                    2 => py_str(&val.str()),
                    3 => {
                        // !a (ascii) conversion: repr() with non-ASCII escaped
                        let s = val.repr();
                        let escaped: String = s.chars().flat_map(|c| {
                            if c.is_ascii() {
                                c.to_string().chars().collect::<Vec<_>>()
                            } else {
                                c.escape_unicode().collect::<Vec<_>>()
                            }
                        }).collect();
                        py_str(&escaped)
                    }
                    _ => return Err(PyError::runtime_error("unknown conversion type")),
                };
                self.frames[fi].push(result);
            }

            Opcode::LOAD_LOCALS => {
                self.frames[fi].push(py_dict());
            }

            Opcode::SETUP_ANNOTATIONS => {}

            Opcode::POP_ITER => {
                self.frames[fi].pop()?;
            }

            Opcode::SETUP_WITH => {
                // Look up __enter__ and call it, keeping manager on stack
                let mgr = self.frames[fi].peek(0)?;
                let _exit_method = mgr.borrow().get_attribute("__exit__").ok();
                let enter_raw = mgr.borrow().get_attribute("__enter__").ok();
                if let Some(enter_raw) = enter_raw {
                    let is_builtin = matches!(&*enter_raw.borrow(), PyObject::BuiltinMethod { .. });
                    let enter = if is_builtin {
                        let b = enter_raw.borrow();
                        match &*b {
                            PyObject::BuiltinMethod { name, func, .. } => {
                                PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func: *func,
                                    self_obj: mgr.clone(),
                                })
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        PyObjectRef::imm(PyObject::BoundMethod {
                            func: enter_raw,
                            self_obj: mgr.clone(),
                        })
                    };
                    let result = self.call_function(enter, vec![], vec![])?;
                    self.frames[fi].push(result);
                } else {
                    self.frames[fi].push(py_none());
                }
            }

            Opcode::WITH_EXIT => {
                // Stack: [..., exception_obj, manager]
                // Call manager.__exit__(exc_type, exc_val, traceback) — exc_type
                // and exc_val must be the real class object and exception
                // instance (not a bare type-name string / the first ctor arg),
                // since __exit__ implementations commonly do isinstance(value,
                // ...), re-raise `value`, or read value.args/__traceback__.
                let mgr = self.frames[fi].pop()?;
                let (typ_obj, val) = {
                    let exc = self.frames[fi].peek(0)?;
                    let exc_borrowed = exc.borrow();
                    match &*exc_borrowed {
                        PyObject::Exception { typ, .. } => {
                            let typ_obj = self.frames[fi].builtins.get(&interner::intern(&typ)).cloned()
                                .unwrap_or_else(|| py_str(typ));
                            (typ_obj, exc.clone())
                        }
                        // A user-defined exception CLASS instance (`class
                        // MyError(Exception): ...`, `raise MyError(...)`)
                        // only ever matched the native `PyObject::Exception`
                        // arm above, silently falling through to `(None,
                        // None)` here — meaning `__exit__(exc_type,
                        // exc_value, tb)` was ALWAYS called as if no
                        // exception had occurred whenever the `with` body
                        // raised a custom exception class, not just the
                        // handful of natively-represented ones. Same root
                        // gap as `CHECK_EXC_MATCH`/`isinstance`/`issubclass`
                        // (already fixed elsewhere this session for their
                        // own call sites) — this is the `with`-statement's
                        // own, previously-unfixed instance of it. Real
                        // trigger: `unittest`'s own `assertRaises`, whose
                        // `_AssertRaisesBaseContext.__exit__` checks `if
                        // exc_type is None: <fail: "X not raised">` — ANY
                        // `assertRaises(CustomExceptionClass, ...)` call
                        // spuriously reported the exception as never having
                        // been raised at all, even though it genuinely was.
                        PyObject::Instance { typ, .. } => (typ.clone(), exc.clone()),
                        _ => (py_none(), py_none()),
                    }
                };
                let exit_raw = mgr.borrow().get_attribute("__exit__")
                    .map_err(|_| PyError::attribute_error("context manager has no __exit__"))?;
                // A method found directly on a native type (e.g. `lock.
                // __exit__`, `Lock`'s attribute lookup in `attrs.rs`) comes
                // back as a `BuiltinMethod` with a PLACEHOLDER `self_obj`
                // (see `NATIVE_VALUE_CTOR_KEY`'s doc comment) — wrapping
                // THAT placeholder-carrying value inside another
                // `BoundMethod{self_obj: mgr}` (the old code here) never
                // actually rebinds it to the real manager: `mgr` never
                // reaches the native implementation, so e.g. `Lock.__exit__`
                // silently no-ops instead of clearing the lock flag, and
                // ANY subsequent `with lock:` on the SAME lock hangs forever
                // spinning on a flag that can never become false again.
                // Mirrors the exact unwrap-and-rebuild-with-the-real-
                // self_obj pattern `SETUP_WITH`'s own `__enter__` handling
                // (just above) already uses — `WITH_EXIT` needs the
                // identical treatment, not a second, ineffective wrapping.
                let is_builtin = matches!(&*exit_raw.borrow(), PyObject::BuiltinMethod { .. });
                let bound = if is_builtin {
                    let b = exit_raw.borrow();
                    match &*b {
                        PyObject::BuiltinMethod { name, func, .. } => {
                            PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: *func,
                                self_obj: mgr.clone(),
                            })
                        }
                        _ => unreachable!(),
                    }
                } else {
                    PyObjectRef::imm(PyObject::BoundMethod {
                        func: exit_raw,
                        self_obj: mgr,
                    })
                };
                let result = self.call_function(bound, vec![typ_obj, val, py_none()], vec![])?;
                self.frames[fi].push(result);
            }

            Opcode::YIELD_VALUE => {
                let val = self.frames[fi].pop()?;
                // Don't push a placeholder; the Generator/Coroutine send method
                // will push the actual sent value (or None for __next__) onto
                // the frame stack, making it available for the next instruction.
                return Ok(Some(val));
            }

            Opcode::RETURN_GENERATOR => {
                // Create a Generator or Coroutine wrapping current frame
                let is_coroutine = self.frames[fi].code.flags & 0x100 != 0;
                let frame = self.frames[fi].clone();
                if is_coroutine {
                    let gen = PyObjectRef::new(PyObject::Coroutine {
                        frame: std::cell::RefCell::new(Some(Box::new(frame))),
                    });
                    return Ok(Some(gen));
                } else {
                    let gen = PyObjectRef::new(PyObject::Generator {
                        frame: std::cell::RefCell::new(Some(Box::new(frame))),
                    });
                    return Ok(Some(gen));
                }
            }

            Opcode::GET_AWAITABLE => {
                // Call __await__ on the object to get an iterator
                let obj = self.frames[fi].pop()?;
                let await_method = obj.borrow().get_attribute("__await__")
                    .map_err(|_| PyError::type_error("object does not support __await__"))?;
                // `get_attribute` on a `Coroutine`/`Generator` returns
                // `__await__` as a `BuiltinMethod` with a `None` PLACEHOLDER
                // `self_obj` (it has no access to the enclosing `PyObjectRef`
                // from inside `impl PyObject`'s `&self`-only method) — its
                // closure body (`|args| Ok(args[0].clone())`, i.e. "return
                // self") then returned that placeholder `None` instead of
                // the real coroutine, so EVERY `await some_async_fn()` where
                // `some_async_fn` itself awaits something else (i.e. any
                // nested async call — confirmed via the simplest possible
                // repro, `async def foo(): return 1` / `async def main():
                // return await foo()`) pushed `None` as the "awaitable
                // iterator" instead of the coroutine, which the immediately
                // following `SEND` then rejected with `TypeError: SEND on
                // non-generator/coroutine/instance` — a fundamental,
                // previously-undetected break in the single most common
                // async/await pattern there is. Rebind `self_obj` to the
                // REAL object here before calling, exactly like the `SEND`
                // opcode handler already does for `send`/`throw` just below
                // (the established fix for this exact class of gap).
                let await_method = match &*await_method.borrow() {
                    PyObject::BuiltinMethod { name, func, .. } => {
                        PyObjectRef::imm(PyObject::BuiltinMethod { name: name.clone(), func: *func, self_obj: obj.clone() })
                    }
                    _ => await_method.clone(),
                };
                let result = self.call_function(await_method, vec![], vec![])?;
                self.frames[fi].push(result);
            }

            Opcode::SEND => {
                // Send value into generator/coroutine/iterator: pop value, peek iterator
                let val = self.frames[fi].pop()?;
                let iter_val = self.frames[fi].peek(0)?;
                let result = {
                    // Try to find a send method on the iterator
                    let is_gen = matches!(&*iter_val.borrow(), PyObject::Generator { .. });
                    let is_coro = matches!(&*iter_val.borrow(), PyObject::Coroutine { .. });
                    if is_gen || is_coro {
                        let method_name = "send";
                        match iter_val.borrow().get_attribute(method_name) {
                            Ok(send_method) => {
                                let bound = match &*send_method.borrow() {
                                    PyObject::BuiltinMethod { func, .. } => {
                                        PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "send".to_string(),
                                            func: *func,
                                            self_obj: iter_val.clone(),
                                        })
                                    }
                                    _ => return Err(PyError::runtime_error("expected BuiltinMethod for send")),
                                };
                                self.call_function(bound, vec![val], vec![])
                            }
                            Err(_) => Err(PyError::attribute_error("object has no send method")),
                        }
                    } else {
                        // Handle Instance objects and other types with a send method
                        match iter_val.borrow().get_attribute("send") {
                            Ok(send_method) => {
                                let bound = match &*send_method.borrow() {
                                    PyObject::BuiltinMethod { func, .. } => {
                                        PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "send".to_string(),
                                            func: *func,
                                            self_obj: iter_val.clone(),
                                        })
                                    }
                                    _ => return Err(PyError::runtime_error("expected BuiltinMethod for send")),
                                };
                                self.call_function(bound, vec![val], vec![])
                            }
                            Err(_) => {
                                // No send method — try __next__ (for simple iterators used with await)
                                Err(PyError::type_error("SEND on non-generator/coroutine/instance"))
                            }
                        }
                    }
                };
                match result {
                    Ok(val) => {
                        self.frames[fi].push(val);
                    }
                    Err(e) => {
                        match e {
                            PyError::StopIteration => {
                                // StopIteration with no value — push None as return value
                                self.frames[fi].push(py_none());
                                // Jump to cleanup target (absolute jump, like FOR_ITER)
                                self.frames[fi].ip = arg as usize;
                            }
                            PyError::Exception(ref typ, ref _exc_val) if typ == "StopIteration" => {
                                // Extract the return value from the PyError::Exception
                                let return_val = _exc_val.clone();
                                self.frames[fi].push(return_val);
                                // Jump to cleanup target (absolute jump, like FOR_ITER)
                                self.frames[fi].ip = arg as usize;
                            }
                            other => return Err(other),
                        }
                    }
                }
            }

            Opcode::END_SEND => {
                // Pop result and iterator, push result (validates proper stack state)
                let result = self.frames[fi].pop()?;
                let _iter = self.frames[fi].pop()?; // iterator, discarded
                self.frames[fi].push(result);
            }

            Opcode::CLEANUP_THROW => {
                // Cleanup after a throw into a generator
                // For now, just a no-op that handles cleanup
                self.frames[fi].pop()?;
            }

            Opcode::ELSE => {
                // No-op marker: separates except handlers from else block.
                // The compiler emits this so the exception table knows where
                // the else block starts.
            }

            Opcode::END_FINALLY => {
                // End of finally block. The stack has either:
                //   [..., value]  — normal execution (no exception)
                //   [..., exc]    — exception was handled, just re-raise
                //   [..., None]   — exception was suppressed/returned
                // We pop the top value. If it's an exception object, re-raise.
                match self.frames[fi].pop() {
                    Ok(val) => {
                        let is_exception = matches!(&*val.borrow(), PyObject::Exception { .. });
                        if is_exception {
                            return Err(PyError::Exception("".to_string(), val));
                        }
                        // Otherwise it was a normal value (or None) — continue
                    }
                    Err(e) => return Err(e),
                }
            }

            Opcode::POP_EXCEPT_AND_EXECUTE_FINALLY => {
                // Popped from POP_EXCEPT: the exception info was already popped.
                // Jump to the finally block address (stored in arg).
                // The finally block address is stored in the `arg` field.
                self.frames[fi].ip = arg as usize;
            }

            Opcode::CALL_FUNCTION_EX => {
                // f(*args, **kwargs) — the compiler already built a real
                // tuple (unpacking any starred arguments via LIST_EXTEND)
                // and a real dict (merging any bare **expr via DICT_MERGE),
                // so this just needs to unpack those into a normal call.
                let kwargs_dict = self.frames[fi].pop()?;
                let args_tuple = self.frames[fi].pop()?;
                let callable = self.frames[fi].pop()?;
                let args_vec = match &*args_tuple.borrow() {
                    PyObject::Tuple(v) | PyObject::List(v) => v.clone(),
                    _ => return Err(PyError::type_error("argument after * must be an iterable")),
                };
                let keywords_vec: Vec<(String, PyObjectRef)> = match &*kwargs_dict.borrow() {
                    PyObject::Dict(d) => d.items().into_iter().map(|(k, v)| (k.str(), v)).collect(),
                    _ => Vec::new(),
                };
                let result = self.call_function(callable, args_vec, keywords_vec)?;
                self.frames[fi].push(result);
            }

            _ => return Err(PyError::runtime_error(format!("unimplemented opcode: {:?}", op))),
        }
        Ok(None)
    }

    /// Resolves `name` on an `Instance` object via its type/MRO (NOT its own
    /// `__dict__` — callers check that themselves first, matching instance-
    /// dict-over-non-data-descriptor precedence), applying the full
    /// descriptor protocol: `property` getters, `staticmethod`/`classmethod`
    /// unwrapping/binding, plain-function-to-bound-method binding, and a
    /// generic `__get__` call for any other descriptor. This mirrors LOAD_ATTR's
    /// own inline logic (kept separate/duplicated rather than shared, to avoid
    /// touching that hot, delicate opcode path) — used by `getattr()`'s
    /// special-case below so it stops returning raw, un-invoked descriptors
    /// (confirmed general: `getattr(obj, 'some_property')` returned the
    /// `property` object itself instead of calling its getter).
    pub(crate) fn resolve_descriptor_attr(&mut self, obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
        let typ = if let PyObject::Instance { typ, .. } = &*obj.borrow() { typ.clone() } else { return None; };
        let found = {
            let typ_ref = typ.borrow();
            if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                type_dict.get_str(name).cloned().or_else(|| {
                    for base in mro.iter().skip(1) {
                        if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                            if let Some(val) = base_dict.get_str(name) {
                                return Some(val.clone());
                            }
                        }
                    }
                    None
                })
            } else {
                None
            }
        }?;
        let val_borrowed = found.borrow();
        match &*val_borrowed {
            PyObject::Property(ref d) if d.getter.is_some() => {
                let g = d.getter.clone().unwrap();
                drop(val_borrowed);
                Some(self.call_function(g, vec![obj.clone()], vec![]).unwrap_or_else(|_| found.clone()))
            }
            PyObject::StaticMethod { func } => Some(func.clone()),
            PyObject::ClassMethod { func } => {
                let func_clone = func.clone();
                Some(PyObjectRef::imm(PyObject::BoundMethod { func: func_clone, self_obj: typ.clone() }))
            }
            PyObject::Function(_) => {
                Some(PyObjectRef::imm(PyObject::BoundMethod { func: found.clone(), self_obj: obj.clone() }))
            }
            // NOTE: deliberately NOT auto-binding a bare `PyObject::Closure`
            // here — see the matching (much longer) comment on this same
            // decision at the `LOAD_ATTR` opcode's own copy of this logic.
            // `Closure` is used both for shared, TYPE-level methods needing
            // `self` bound (which should use `BuiltinFunction` instead —
            // that already auto-binds correctly) AND for per-instance
            // closures capturing their own state directly and expecting NO
            // `self` prepended (`io.BytesIO`'s `read`/`write`/`seek`, ...) —
            // auto-binding unconditionally broke the latter.
            PyObject::BuiltinFunction { name: n, .. } if crate::object::is_builtin_exception_class_name(n) => {
                // Don't auto-bind a builtin exception "class" — see the
                // matching LOAD_ATTR fix's own (much longer) comment.
                Some(found.clone())
            }
            PyObject::BuiltinFunction { name: n, func } => {
                Some(PyObjectRef::imm(PyObject::BuiltinMethod { name: n.clone(), func: *func, self_obj: obj.clone() }))
            }
            PyObject::BuiltinMethod { name: n, func, .. } => {
                Some(PyObjectRef::imm(PyObject::BuiltinMethod { name: n.clone(), func: *func, self_obj: obj.clone() }))
            }
            _ => {
                drop(val_borrowed);
                if let Ok(get_fn) = found.borrow().get_attribute("__get__") {
                    let descriptor_args = vec![found.clone(), obj.clone(), typ.clone()];
                    match self.call_function(get_fn, descriptor_args, vec![]) {
                        Ok(v) => Some(v),
                        Err(_) => Some(found.clone()),
                    }
                } else {
                    Some(found.clone())
                }
            }
        }
    }

    pub(crate) fn call_function(&mut self, callable: PyObjectRef, args: Vec<PyObjectRef>, keywords: Vec<(String, PyObjectRef)>) -> PyResult<PyObjectRef> {
        let type_name = callable.borrow().type_name();
        if cfg!(feature = "profile") { eprintln!("DEBUG call_function: type={} name={:?}", type_name, callable.repr()); }
        if std::env::var("RPY_DEBUG_CALL").is_ok() {
            eprintln!("CALL_FUNCTION: type={} repr={}", type_name, callable.repr());
        }

        // `type.__new__` needs live `&mut self` access (to build the class
        // via `default_build_class`, which itself calls back into
        // `self.call_function` for __set_name__/__init_subclass__) — a
        // plain BuiltinFunction can't capture that, and routing it through
        // `with_vm_mut`'s thread-local VM_PTR here would hand back a
        // *second*, aliasing `&mut VirtualMachine` while this exact call
        // chain already holds one (we're inside `&mut self` right now),
        // which is undefined behavior and reliably segfaulted in testing.
        // Special-case it here instead, exactly like `__build_class__`
        // and bare `type` are special-cased above/below, so it goes
        // through the real, single, already-live `self`.
        {
            let is_type_new = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::type_new_builtin as crate::object::BuiltinFunc));
            if is_type_new {
                let mut all_args = args;
                if !keywords.is_empty() {
                    let mut dict = crate::object::PyDict::new();
                    for (k, v) in &keywords {
                        let _ = dict.set(crate::object::py_str(k), v.clone());
                    }
                    all_args.push(PyObjectRef::new(PyObject::Dict(Box::new(dict))));
                }
                return self.type_new_impl(&all_args);
            }
        }

        // `getattr(obj, name[, default])` on a plain `Instance` needs to
        // fall back to the type's `__getattr__` (mro-walked) when the raw
        // lookup fails — the same fallback `LOAD_ATTR`'s own opcode
        // handler already does, but `object::builtin_getattr` (a plain
        // `fn(&[PyObjectRef])`, no VM access) can't call a found
        // `__getattr__` itself. Special-cased here (matching `type.__new__`
        // just above) so it happens through the one real, live `self`
        // instead of `with_vm_mut`/`call_bound_method`'s disposable-VM
        // path — a `__getattr__` doing a lazy import (a real, common
        // pattern to dodge circular imports, same as elsewhere this
        // session) would otherwise silently re-import everything from
        // scratch in an empty module registry. Confirmed general, not
        // Django-specific: any two-level `__getattr__` proxy chain where
        // the outer level's own `__getattr__` calls the builtin
        // `getattr(self._wrapped, name)` (real code: Django's
        // `LazySettings`/`UserSettingsHolder`) hit this — `django.conf.
        // settings.LOGGING_CONFIG` (and every other setting not
        // explicitly passed to `settings.configure()`) failed with a
        // nonsensical "instance has no attribute" instead of falling
        // through to the wrapped default-settings module.
        {
            let is_getattr = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_getattr as crate::object::BuiltinFunc));
            if is_getattr && (args.len() == 2 || args.len() == 3) {
                let obj = args[0].clone();
                let attr_name = args[1].str();
                if std::env::var("RPY_DEBUG_GETATTR").is_ok() {
                    let type_name = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                        if let PyObject::Type { name, .. } = &*typ.borrow() { name.clone() } else { "?".to_string() }
                    } else {
                        obj.borrow().type_name().to_string()
                    };
                    eprintln!("GETATTR: obj_type={} attr={}", type_name, attr_name);
                }
                // Instance's own __dict__ wins over any class-level
                // descriptor (non-data-descriptor precedence). Only past
                // that do we need real descriptor-protocol dispatch —
                // `object::builtin_getattr`'s plain `get_attribute` (the
                // "direct" fallback below) returns raw, un-invoked
                // `property`/custom-`__get__` values otherwise, unlike
                // LOAD_ATTR's own opcode handler. Confirmed general via a
                // Django-free repro: `getattr(obj, 'some_property')`
                // returned the `property` object itself instead of calling
                // its getter, and a custom descriptor's `__get__` was
                // never invoked at all.
                let own_dict_hit = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                    dict.get(&attr_name).cloned()
                } else {
                    None
                };
                if let Some(v) = own_dict_hit {
                    return Ok(v);
                }
                if let Some(v) = self.resolve_descriptor_attr(&obj, &attr_name) {
                    return Ok(v);
                }
                let direct = obj.borrow().get_attribute(&attr_name);
                match direct {
                    Ok(v) => {
                        // object.rs's plain get_attribute (used for the
                        // "direct" success path here) doesn't auto-bind a
                        // plain Function found on an Instance into a
                        // BoundMethod — only the LOAD_ATTR opcode's own,
                        // separate logic does that. Without this,
                        // `getattr(instance, name)` for a real method
                        // returns it UNBOUND while `instance.name` (real
                        // attribute syntax) correctly binds it — an
                        // inconsistency that silently drops `self` the
                        // moment calling code relies on `getattr()` instead
                        // of dot access (a common proxy-object idiom, e.g.
                        // `new_method_proxy`-style `__getattr__` forwarding
                        // via `getattr(self._wrapped, name)`).
                        let is_instance_obj = matches!(&*obj.borrow(), PyObject::Instance { .. });
                        let is_function = matches!(&*v.borrow(), PyObject::Function(_));
                        if is_instance_obj && is_function {
                            return Ok(PyObjectRef::imm(PyObject::BoundMethod { func: v, self_obj: obj.clone() }));
                        }
                        // `@classmethod`-decorated attributes found on a
                        // class (`obj` a `PyObject::Type`) come back from
                        // plain `get_attribute` as the raw, un-invoked
                        // `ClassMethod` descriptor — only LOAD_ATTR's own
                        // opcode handler binds it into a callable
                        // `BoundMethod`. Without this, `getattr(SomeClass,
                        // 'a_classmethod')()` raised `TypeError:
                        // 'classmethod' object is not callable` even
                        // though `SomeClass.a_classmethod()` worked fine.
                        // Real trigger: `unittest.suite.py`'s
                        // `getattr(currentClass, 'setUpClass', None)` —
                        // every `TestCase` subclass's default
                        // `@classmethod setUpClass`/`tearDownClass` hit
                        // this the moment `_isnotsuite()` (itself only
                        // fixed to work correctly this same session) let
                        // per-class fixture handling actually run for the
                        // first time.
                        let is_type_obj = matches!(&*obj.borrow(), PyObject::Type { .. });
                        if is_type_obj {
                            if let PyObject::ClassMethod { func } = &*v.borrow() {
                                return Ok(PyObjectRef::imm(PyObject::BoundMethod { func: func.clone(), self_obj: obj.clone() }));
                            }
                        }
                        // Native (non-Instance) types — File, List, Dict,
                        // Set, ... — expose their own methods as
                        // `BuiltinMethod` values with a `PyObject::None`
                        // PLACEHOLDER `self_obj`, meant to always be rebound
                        // to whatever object they were actually looked up
                        // on (LOAD_ATTR's own opcode handling does this
                        // rebinding inline; plain `get_attribute` — used for
                        // this "direct" success path — never did). Without
                        // this, `getattr(some_file, 'write')` (a real,
                        // common proxy idiom — e.g. `unittest`'s own
                        // `_WritelnDecorator.__getattr__` forwarding via
                        // `getattr(self.stream, attr)`) returned a `write`
                        // method still bound to that placeholder `None`,
                        // so calling it failed with "write on non-file".
                        let rebind_builtin_method = if let PyObject::BuiltinMethod { name, func, self_obj } = &*v.borrow() {
                            let placeholder = matches!(&*self_obj.borrow(), PyObject::None);
                            if placeholder && !matches!(&*obj.borrow(), PyObject::Instance { .. }) {
                                Some((name.clone(), *func))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if let Some((name, func)) = rebind_builtin_method {
                            return Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name, func, self_obj: obj.clone() }));
                        }
                        return Ok(v);
                    }
                    Err(direct_err) => {
                        let getattr_fn = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                            crate::object::lookup_dunder_via_mro(typ, "__getattr__")
                        } else {
                            None
                        };
                        if let Some(f) = getattr_fn {
                            match self.call_function(f, vec![obj.clone(), crate::object::py_str(&attr_name)], vec![]) {
                                Ok(v) => return Ok(v),
                                Err(_) if args.len() == 3 => return Ok(args[2].clone()),
                                Err(e) => return Err(e),
                            }
                        }
                        if args.len() == 3 {
                            return Ok(args[2].clone());
                        }
                        return Err(direct_err);
                    }
                }
            }
        }

        // `hasattr(obj, name)` — same descriptor-protocol gap as `getattr`
        // just above (`object::builtin_hasattr`, also a plain `fn(&[PyObjectRef])`
        // with no VM access, can only do raw `get_attribute`): a `property`/
        // custom descriptor whose getter RAISES should make `hasattr` return
        // False (matching real Python's "hasattr calls getattr and catches
        // the exception" semantics), but raw retrieval never invokes the
        // getter at all, so it can never observe that failure.
        {
            let is_hasattr = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_hasattr as crate::object::BuiltinFunc));
            if is_hasattr && args.len() == 2 {
                let obj = args[0].clone();
                let attr_name = args[1].str();
                let own_dict_hit = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                    dict.get(&attr_name).cloned()
                } else {
                    None
                };
                if own_dict_hit.is_some() {
                    return Ok(py_bool(true));
                }
                if self.resolve_descriptor_attr(&obj, &attr_name).is_some() {
                    return Ok(py_bool(true));
                }
                return Ok(py_bool(obj.borrow().get_attribute(&attr_name).is_ok()));
            }
        }

        // `sys.exc_info()` — same `with_vm_mut`-is-unconditional-UB class
        // of bug as the `exec()`/`eval()` fix just below (confirmed via the
        // simplest possible repro: `except Exception: sys.exc_info()`
        // reliably segfaulting). Read the real, live VM's own exception
        // fields directly instead.
        {
            let is_exc_info = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_exc_info_builtin as crate::object::BuiltinFunc));
            if is_exc_info {
                if std::env::var("RPY_DEBUG_EXCINFO").is_ok() {
                    eprintln!("READ exc_info: type={:?} value={:?}", self.exc_type.as_ref().map(|v| v.repr()), self.exc_value.as_ref().map(|v| v.repr()));
                }
                return Ok(py_tuple(vec![
                    self.exc_type.clone().unwrap_or_else(py_none),
                    self.exc_value.clone().unwrap_or_else(py_none),
                    self.exc_traceback.clone().unwrap_or_else(py_none),
                ]));
            }
        }

        // `sys.exception()` (3.11+) — same fix, same reason, as
        // `sys.exc_info()` just above: reads `self.exc_value` directly
        // instead of going through `with_vm_mut`, which gave the wrong
        // (always-empty) result from this reentrant calling context.
        {
            let is_exception = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_exception_builtin as crate::object::BuiltinFunc));
            if is_exception {
                return Ok(self.exc_value.clone().unwrap_or_else(py_none));
            }
        }

        // `sys.getrecursionlimit()`/`setrecursionlimit()` — read/write
        // `self.recursion_limit` directly (same `with_vm_mut`-avoidance
        // pattern as everything else here) instead of the fallback
        // `with_vm_mut`-based native fns, which are otherwise unconditional
        // UB from within a live call chain like every other case on this
        // page.
        {
            let is_getrecursionlimit = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_getrecursionlimit_builtin as crate::object::BuiltinFunc));
            if is_getrecursionlimit {
                return Ok(py_int(self.recursion_limit as i64));
            }
            let is_setrecursionlimit = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_setrecursionlimit_builtin as crate::object::BuiltinFunc));
            if is_setrecursionlimit {
                let n = args.get(0).and_then(|a| a.as_i64()).ok_or_else(|| PyError::type_error("setrecursionlimit() requires an integer argument"))?;
                if n < 1 { return Err(PyError::value_error("recursion limit must be greater or equal than 1")); }
                self.recursion_limit = n as usize;
                return Ok(py_none());
            }
        }

        // `print()` — needs the live VM to look up the CURRENT `sys.stdout`
        // (not a cached reference) and to accept `sep`/`end`/`file`/`flush`
        // keyword arguments, which the generic `BuiltinFunction` dispatch
        // path further below would otherwise pack into a trailing dict
        // ARGUMENT (this project's established kwargs-passing convention
        // for plain builtins) — silently printing that dict as if it were
        // one more thing to print, since the old implementation just joined
        // every element of `args` unconditionally. See `print_with_vm`'s
        // own doc comment for the full story.
        if matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_print as crate::object::BuiltinFunc)) {
            return crate::object::print_with_vm(self, &args, &keywords);
        }

        // `globals()`/`locals()` — same `with_vm_mut`-is-unconditional-UB
        // class of bug (confirmed via a general repro: `def f(): locals()`
        // — not a segfault this time, but `vm.frames` reading back empty
        // through the aliased pointer, "RuntimeError: no frame", even
        // though the real VM's frame stack plainly wasn't empty). Read
        // `self.frames` directly instead of going through `with_vm_mut`.
        {
            let is_globals = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_globals as crate::object::BuiltinFunc));
            let is_locals = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_locals as crate::object::BuiltinFunc));
            if is_globals || is_locals {
                let frame = self.frames.last().ok_or_else(|| PyError::runtime_error("no frame"))?;
                let mut d = crate::object::PyDict::new();
                if is_globals {
                    for (k, v) in frame.globals.borrow().iter() {
                        d.set(py_str(interner::lookup_str(*k)), v.clone())?;
                    }
                } else {
                    // Merge fast-locals (function-scope named params/vars,
                    // keyed by position against `code.varnames`) with the
                    // name-keyed `locals` map (module/class-scope variables,
                    // which never go through STORE_FAST at all) — a real
                    // snapshot needs both; the pre-fix version only ever
                    // read the latter, so a function's own locals() always
                    // came back empty regardless of the frame lookup bug.
                    for (i, slot) in frame.fast_locals.iter().enumerate() {
                        if let Some(v) = slot {
                            if let Some(&name) = frame.code.varnames.get(i) {
                                d.set(py_str(crate::interner::lookup_str(name)), v.clone())?;
                            }
                        }
                    }
                    for (k, v) in frame.locals.iter() {
                        let name = crate::interner::lookup(k);
                        d.set(py_str(&name), v.clone())?;
                    }
                }
                return Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))));
            }
        }

        // `sys._getframe(depth=0)` — was a complete no-op stub, always
        // returning `None` regardless of `depth` (`object::core.rs`'s
        // version has no VM access at all to do otherwise). Real trigger:
        // `Lib/test/support/warnings_helper.py`'s `_filterwarnings`
        // (`sys._getframe(2)`, to clear the CALLING module's
        // `__warningregistry__` so warnings can be re-raised) — used by
        // `check_warnings`, itself used pervasively across the corpus by
        // any test asserting on warning behavior. Same `with_vm_mut`-
        // avoidance pattern as `globals()`/`locals()` just above: reads
        // `self.frames` directly. Returns a minimal but real `frame`-shaped
        // `Instance` exposing `f_globals` as a live dict snapshot (each
        // VALUE is the same shared `PyObjectRef` as the frame's real
        // globals entry, so mutating a nested container — e.g. clearing
        // `__warningregistry__` — still affects the real frame, even
        // though the snapshot dict itself is a fresh copy) — enough for
        // this and similar introspection uses, not a full frame object.
        {
            let is_getframe = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_getframe_builtin as crate::object::BuiltinFunc));
            if is_getframe {
                let depth = args.first().and_then(|a| a.as_i64()).unwrap_or(0);
                if depth < 0 {
                    return Err(PyError::value_error("call stack is not deep enough"));
                }
                let idx = (self.frames.len() as i64) - 1 - depth;
                let frame = if idx >= 0 { self.frames.get(idx as usize) } else { None };
                let frame = frame.ok_or_else(|| PyError::value_error("call stack is not deep enough"))?;
                let mut fg = crate::object::PyDict::new();
                for (k, v) in frame.globals.borrow().iter() {
                    fg.set(py_str(interner::lookup_str(*k)), v.clone())?;
                }
                let mut attrs = crate::object::AttrMap::new();
                attrs.insert_str("f_globals", PyObjectRef::new(PyObject::Dict(Box::new(fg))));
                attrs.insert_str("f_code", PyObjectRef::imm(PyObject::Code(frame.code.clone())));
                let typ = PyObjectRef::new(PyObject::Type { name: "frame".to_string(), dict: Box::new(crate::object::TypeDict::default()), bases: vec![], mro: vec![] });
                return Ok(PyObjectRef::new(PyObject::Instance { typ, dict: attrs }));
            }
        }

        // `isinstance(obj, cls)`/`issubclass(sub, cls)` — real Python lets a
        // custom METACLASS override these entirely by defining its own
        // `__instancecheck__`/`__subclasscheck__` (distinct from, and rarer
        // than, `__subclasshook__`-based ABC registration, which the
        // generic `builtin_isinstance`/`builtin_issubclass` dispatch
        // already supports elsewhere). Real trigger: CPython's own
        // `test_typechecks.py` (`class ABC(type): def __instancecheck__
        // (cls, inst): ...`). `object::builtin_isinstance`/
        // `builtin_issubclass` are plain `fn(&[PyObjectRef])` with no VM
        // access, so they can never CALL such a hook — only special-cased
        // here, with the real, live `self`, and only when a custom
        // metaclass hook is actually present (checked cheaply up front);
        // falls through to the normal, unmodified dispatch otherwise, so
        // the overwhelmingly common no-custom-metaclass path is completely
        // unaffected. Handles the tuple-of-classes form too (`isinstance
        // (x, (A, B))`) directly here, since `builtin_isinstance`'s OWN
        // internal tuple recursion is a plain Rust call that never reaches
        // this dispatch layer for each member.
        {
            let is_isinstance = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_isinstance as crate::object::BuiltinFunc));
            let is_issubclass = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_issubclass as crate::object::BuiltinFunc));
            if (is_isinstance || is_issubclass) && args.len() == 2 {
                let hook_name = if is_isinstance { "__instancecheck__" } else { "__subclasscheck__" };
                let find_hook = |cls: &PyObjectRef| -> Option<PyObjectRef> {
                    if !matches!(&*cls.borrow(), PyObject::Type { .. }) { return None; }
                    let mt = crate::object::metatype_of(cls)?;
                    let hook = if let PyObject::Type { dict, .. } = &*mt.borrow() { dict.get_str(hook_name).cloned() } else { None };
                    hook
                };
                let classes: Vec<PyObjectRef> = match &*args[1].borrow() {
                    PyObject::Tuple(items) => items.clone(),
                    _ => vec![args[1].clone()],
                };
                if classes.iter().any(|c| find_hook(c).is_some()) {
                    for cls in &classes {
                        if let Some(hook) = find_hook(cls) {
                            let bound = PyObjectRef::imm(PyObject::BoundMethod { func: hook, self_obj: cls.clone() });
                            let result = self.call_function(bound, vec![args[0].clone()], vec![])?;
                            if result.truthy() {
                                return Ok(py_bool(true));
                            }
                        } else if is_isinstance {
                            if crate::object::builtin_isinstance(&[args[0].clone(), cls.clone()])?.truthy() {
                                return Ok(py_bool(true));
                            }
                        } else if crate::object::builtin_issubclass(&[args[0].clone(), cls.clone()])?.truthy() {
                            return Ok(py_bool(true));
                        }
                    }
                    return Ok(py_bool(false));
                }
            }
        }

        // `__import__(name, ...)` — what every `import` STATEMENT desugars
        // to in real CPython; this interpreter's own `IMPORT_NAME` opcode
        // doesn't route through it, but plenty of real code calls it
        // explicitly (confirmed segfaulting via the simplest possible
        // repro, `__import__("os")` at plain top level — same
        // `with_vm_mut`-is-unconditional-UB class of bug as `exec`/`eval`/
        // `sys.exc_info()`/`globals()`/`locals()` above). Shares
        // `object::import_impl` (extracted out of `builtin_import` for
        // exactly this) with the real VM directly.
        {
            let is_import = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_import as crate::object::BuiltinFunc));
            // Real `__import__`'s `name` is commonly passed as a KEYWORD
            // argument too (`__import__(name='sys')` — exactly what
            // `test_builtin.py::BuiltinTest.test_import` exercises). Since
            // `keywords` arrives as a SEPARATE parameter here (not yet
            // packed into `args`), the old `!args.is_empty()` guard was
            // false whenever `name` was keyword-only, silently falling
            // through to `object::builtin_import`'s generic `with_vm_mut`
            // path below — which then treats the whole packed kwargs DICT
            // as the module name (stringifying it to garbage like
            // `"{'name': 'sys'}"`) and feeds that into the import
            // machinery, corrupting `self.modules`'s backing allocation
            // (confirmed via `gdb`: SIGSEGV inside a `HashMap::get("sys")`
            // call in `get_sys_path`, reached via the very same
            // `with_vm_mut` raw-pointer-aliasing UB class documented
            // throughout this function) rather than raising a clean error.
            let name_kw = keywords.iter().find(|(k, _)| k == "name").map(|(_, v)| v.clone());
            if is_import && (!args.is_empty() || name_kw.is_some()) {
                // Real CPython rejects `name` given BOTH positionally and by
                // keyword (`__import__('sys', name='sys')`) with a
                // `TypeError` — `test_builtin.py::BuiltinTest.test_import`
                // checks this exact case too.
                if !args.is_empty() && name_kw.is_some() {
                    return Err(PyError::type_error("argument for __import__() given by name ('name') and position (1)"));
                }
                let name_obj = args.get(0).cloned().or(name_kw).unwrap();
                // Real `__import__` requires `name` to actually be a `str`
                // (`__import__(1, 2, 3, 4)` — exercised directly by
                // `test_builtin.py::BuiltinTest.test_import` — must raise
                // `TypeError`, not silently coerce the int via `.str()` and
                // go looking for a module literally named `"1"`).
                if !matches!(&*name_obj.borrow(), PyObject::Str(_)) {
                    return Err(PyError::type_error("__import__() argument 'name' must be str"));
                }
                let name = name_obj.str();
                // `__import__('')` (empty module name) is a real
                // `ValueError` in CPython, not "module not found" — but
                // ONLY for an absolute import (`level=0`, the default).
                // With `level>0` an empty name is the NORMAL, valid
                // encoding of a pure relative import (`from . import foo`
                // desugars to `__import__('', globals(), locals(),
                // ['foo'], 1)`) — `test_builtin.py::BuiltinTest.test_import`
                // exercises both: `__import__('')` (level 0, expects
                // ValueError) and a `level=1` call with fromlist (expects
                // ImportError from the relative-import-with-no-package
                // check, not ValueError).
                let level_kw = keywords.iter().find(|(k, _)| k == "level").map(|(_, v)| v.clone());
                let level = args.get(4).cloned().or(level_kw).and_then(|v| v.as_i64()).unwrap_or(0);
                if name.is_empty() && level == 0 {
                    return Err(PyError::value_error("Empty module name"));
                }
                // `fromlist` is overwhelmingly passed as a KEYWORD argument
                // in real code (`__import__(name, fromlist=[...])` — real
                // trigger: CPython's own `dbm/__init__.py`), which under
                // this project's calling convention arrives as a trailing
                // packed kwargs dict, not a 4th positional argument — see
                // `object::builtin_import`'s matching doc comment for the
                // full story (checking only `args[3]` silently always
                // returned the top-level package instead of the requested
                // submodule, an infinite-recursion trap for callers that
                // then call `.open`/etc. on what they assumed was the
                // specific submodule).
                let kwargs_fromlist = keywords.iter().find(|(k, _)| k == "fromlist").map(|(_, v)| v.clone());
                let fromlist_arg = kwargs_fromlist.or_else(|| args.get(3).cloned());
                let fromlist = fromlist_arg.and_then(|fl| {
                    match &*fl.borrow() {
                        PyObject::List(items) => Some(items.clone()),
                        PyObject::Tuple(items) => Some(items.iter().cloned().collect::<Vec<_>>()),
                        _ => None,
                    }
                });
                let has_dots = name.contains('.');
                let has_fromlist = fromlist.as_ref().map_or(false, |fl: &Vec<PyObjectRef>| !fl.is_empty());
                return crate::object::import_impl(self, &name, has_dots, has_fromlist);
            }
        }

        // `asyncio.run(coro)` — same `with_vm_mut`-is-unconditional-UB class
        // of bug (confirmed segfaulting via the simplest possible repro:
        // `asyncio.run(some_async_def())`, an extremely common real-world
        // async entry point). Shares `modules::asyncio_run_impl` (extracted
        // out of the inline closure for exactly this) with the real VM
        // directly.
        {
            let is_asyncio_run = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::asyncio_run_builtin as crate::object::BuiltinFunc));
            if is_asyncio_run && !args.is_empty() {
                return crate::modules::asyncio_run_impl(self, args[0].clone());
            }
        }

        // `signal.raise_signal(signum)` / `os.kill(pid, signum)` (own pid
        // only — the only pid meaningful in this single-process
        // interpreter) — actually CALLING a registered `signal.signal()`
        // handler needs a live `&mut VirtualMachine` (same class of bug as
        // `asyncio.run`/`start_new_thread` above). Confirmed via
        // `test_threadsignals.py`'s `acquire_retries_on_intr`, which relies
        // on `os.kill(os.getpid(), signal.SIGUSR1)` actually invoking the
        // handler registered via `signal.signal(signal.SIGUSR1, my_handler)`.
        {
            let is_raise_signal = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::signal_raise_signal_builtin as crate::object::BuiltinFunc));
            if is_raise_signal && !args.is_empty() {
                let signum = args[0].as_i64().ok_or_else(|| PyError::type_error("signalnum must be an int"))?;
                return crate::modules::signal_raise_signal_impl(self, signum);
            }
            let is_os_kill = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::os_kill_builtin as crate::object::BuiltinFunc));
            if is_os_kill && args.len() >= 2 {
                let pid = args[0].as_i64().unwrap_or(-1);
                let signum = args[1].as_i64().ok_or_else(|| PyError::type_error("signalnum must be an int"))?;
                if pid == std::process::id() as i64 {
                    crate::modules::invoke_signal_handler_impl(self, signum)?;
                }
                return Ok(py_none());
            }
        }

        // `exec(source[, globals[, locals]])` / `eval(source[, globals[, locals]])`
        // — `object::builtin_exec`/`builtin_eval` (plain `fn(&[PyObjectRef])`,
        // no VM access) reached the VM via `with_vm_mut`, which grabs the
        // SAME `*mut VirtualMachine` this call is already executing under —
        // real aliasing UB (a second live `&mut self` to an object already
        // mutably borrowed by the current Rust call stack), not just "risky
        // in theory". `VM_PTR` is set unconditionally in `execute()` before
        // ANY bytecode runs, so this UB was hit by every `exec()`/`eval()`
        // call from normal running Python code, not just some rare nested
        // case — confirmed via the simplest possible repro (`exec("x = 1")`
        // at plain top level) reliably segfaulting. Fixed the same way as
        // `getattr`/`hasattr`/etc. above: run it through the real, live
        // `self` directly. Also fixes real semantics `with_vm_mut`'s
        // `vm.run(code)` never had: an explicit `globals`/`locals` dict
        // argument (needed by real code that generates functions via
        // `exec(src, globals_dict, locals_dict)` — CPython's own
        // `dataclasses.py` does exactly this) is now actually honored
        // instead of always executing against the top-level module globals.
        {
            let is_exec = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_exec as crate::object::BuiltinFunc));
            let is_eval = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_eval as crate::object::BuiltinFunc));
            if (is_exec || is_eval) && !args.is_empty() {
                let mode_name = if is_exec { "exec" } else { "eval" };
                let code = match &*args[0].borrow() {
                    PyObject::Code(c) => (**c).clone(),
                    _ => {
                        let source = args[0].str();
                        // `eval()` compiles as a single EXPRESSION (returns
                        // its value via RETURN_VALUE) — `exec()` compiles as
                        // a statement list (returns None, matching module-
                        // level execution); using statement-mode for both
                        // (the pre-fix code's bug) made `eval("2+2")` return
                        // None instead of 4.
                        // A real `SyntaxError` (not `TypeError`) — see
                        // `PyError::syntax_error`'s own doc comment; same
                        // fix as `builtin_compile`'s equivalent parse sites.
                        let program = if is_eval {
                            crate::parser::try_parse_as_expression(&source).map_err(PyError::syntax_error)?
                        } else {
                            let mut parser = crate::parser::Parser::new(&source);
                            parser.parse_program().map_err(PyError::syntax_error)?
                        };
                        let mut compiler = crate::compiler::Compiler::new();
                        compiler.compile(&program, &format!("<{}>", mode_name)).map_err(PyError::syntax_error)?
                    }
                };
                // Merge an explicit globals dict (reads) with an explicit
                // locals dict (reads take precedence, writes land here) into
                // one flat namespace — this interpreter's frames don't model
                // separate globals/locals scopes for top-level-style exec.
                let globals_dict = args.get(1).filter(|g| matches!(&*g.borrow(), PyObject::Dict(_)));
                let locals_dict = args.get(2).filter(|l| matches!(&*l.borrow(), PyObject::Dict(_))).or(globals_dict);
                let namespace = if let Some(g) = globals_dict {
                    let mut hm: HashMap<StrId, PyObjectRef> = str_map_to_strid_map(crate::object::dict_arg_to_hashmap(g, "exec() globals must be a dict")?);
                    if let Some(l) = args.get(2).filter(|l| matches!(&*l.borrow(), PyObject::Dict(_))) {
                        hm.extend(str_map_to_strid_map(crate::object::dict_arg_to_hashmap(l, "exec() locals must be a dict")?));
                    }
                    Some(Rc::new(RefCell::new(hm)))
                } else {
                    None
                };
                let globals_rc = namespace.clone().unwrap_or_else(|| self.frames.last().map(|f| f.globals.clone()).unwrap_or_else(|| self.globals.clone()));
                let result = self.exec_code(code, Some(globals_rc.clone()));
                if let Some(target) = locals_dict {
                    if let PyObject::Dict(d) = &mut *target.borrow_mut() {
                        for (k, v) in globals_rc.borrow().iter() {
                            let _ = d.set(py_str(interner::lookup_str(*k)), v.clone());
                        }
                    }
                }
                return match result {
                    Ok(val) => Ok(if is_exec { py_none() } else { val }),
                    Err(e) => Err(e),
                };
            }
        }

        // `importlib.import_module(name, package=None)` — same reasoning
        // as `getattr` just above: its own implementation normally reaches
        // the VM only via `with_vm_mut`, a second aliasing `&mut self`
        // while this exact call chain already holds one. Real code calls
        // this constantly (Django's own `django.utils.module_loading.
        // import_string` — used to resolve `LOGGING_CONFIG =
        // "logging.config.dictConfig"` and similar dotted-path settings —
        // goes through `importlib.import_module` for the module half of
        // the path), so route it through the live `self` directly instead.
        {
            let is_import_module = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::import_module_builtin as crate::object::BuiltinFunc));
            if is_import_module && !args.is_empty() {
                let name = args[0].str();
                let package = if args.len() >= 2 {
                    let pkg = args[1].str();
                    if pkg.is_empty() { None } else { Some(pkg) }
                } else { None };
                return crate::modules::import_module_with_vm(self, &name, package.as_deref());
            }
        }

        // `importlib.util.find_spec` (`find_spec_builtin`) internally used
        // `with_vm_mut` to read `vm.modules`/`sys.path` — reached constantly
        // from deep inside an active VM call chain in practice (e.g. Django's
        // app registry calls it while `apps.populate()` is running), which
        // reborrows the *same* live VirtualMachine `with_vm_mut` already has
        // a `&mut self` for elsewhere on the Rust call stack: real aliasing
        // UB, confirmed via a non-deterministic segfault/corrupted-HashMap
        // crash (not just theoretical). Route it through the real, live
        // `&mut self` directly instead, same pattern as getattr/import_module
        // above.
        {
            let is_find_spec = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::find_spec_builtin as crate::object::BuiltinFunc));
            if is_find_spec && !args.is_empty() {
                let name = args[0].str();
                let package = if args.len() >= 2 {
                    let pkg = args[1].str();
                    if pkg.is_empty() { None } else { Some(pkg) }
                } else { None };
                return crate::modules::find_spec_with_vm(self, &name, package.as_deref());
            }
        }

        // `inspect.getmembers(obj, predicate)` needs to actually CALL
        // `predicate` on each candidate member — same reentrancy hazard as
        // find_spec above (reached from deep inside Django's app-loading:
        // `inspect.getmembers(mod, inspect.isclass)`), so route it through
        // the real, live `&mut self` directly instead of a disposable VM.
        {
            let is_getmembers = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::getmembers_builtin as crate::object::BuiltinFunc));
            if is_getmembers && !args.is_empty() {
                let predicate = args.get(1).cloned();
                return crate::modules::getmembers_with_vm(self, &args[0], predicate.as_ref());
            }
        }

        if let PyObject::BuiltinFunction { func, .. } = &*callable.borrow() {
            let func = *func;
            // Pack keyword arguments into a dict and append as last arg
            if !keywords.is_empty() {
                let mut dict = crate::object::PyDict::new();
                for (k, v) in &keywords {
                    let _ = dict.set(crate::object::py_str(k), v.clone());
                }
                let mut new_args = args;
                new_args.push(crate::object::PyObjectRef::new(crate::object::PyObject::Dict(Box::new(dict))));
                return func(&new_args);
            }
            return func(&args);
        }

        if let PyObject::BuiltinMethod { func, self_obj, .. } = &*callable.borrow() {
            let func = *func;
            let self_obj = self_obj.clone();
            let mut new_args = vec![self_obj];
            new_args.extend(args);
            if !keywords.is_empty() {
                let mut dict = crate::object::PyDict::new();
                for (k, v) in keywords {
                    let _ = dict.set(crate::object::py_str(&k), v);
                }
                new_args.push(crate::object::PyObjectRef::new(crate::object::PyObject::Dict(Box::new(dict))));
            }
            // `generator.throw()` needs real `&mut self` access so the
            // resumed generator body's `sys.exc_info()` sees THIS VM's
            // exc_type/exc_value (set moments earlier by the original
            // `raise`) instead of a disposable VM's blank ones — see
            // `generator_throw_with_vm`'s own doc comment.
            if std::ptr::fn_addr_eq(func, crate::object::generator_throw_fallback as crate::object::BuiltinFunc) {
                return crate::object::generator_throw_with_vm(self, &new_args);
            }
            return func(&new_args);
        }

        if let PyObject::BoundMethod { func, self_obj } = &*callable.borrow() {
            let func = func.clone();
            let self_obj = self_obj.clone();
            let mut new_args = vec![self_obj];
            new_args.extend(args);
            return self.call_function(func, new_args, keywords);
        }

        if let PyObject::Partial { func, args: partial_args } = &*callable.borrow() {
            let func = func.clone();
            let mut all_args = partial_args.clone();
            all_args.extend(args);
            return self.call_function(func, all_args, keywords);
        }

        #[cfg_attr(not(feature = "jit"), allow(unused_variables))]
        if let PyObject::Function(ref inner_f) = &*callable.borrow() {
            let code = &inner_f.code;
            let func_globals = &inner_f.globals;
            let defaults = &inner_f.defaults;
            let closure = &inner_f.closure;
            let jit_ptr = &inner_f.jit_ptr;
            let jit_consts = &inner_f.jit_consts;
            // Try JIT compiled execution (fast path for hot functions)
            #[cfg(feature = "jit")]
            if defaults.is_empty() && keywords.is_empty() {
                const SENTINEL_FAILED: usize = 1;
                let jp = jit_ptr.get();
                if jp == 0 {
                    // First call: try to compile; set sentinel so we don't retry
                    jit_ptr.set(SENTINEL_FAILED);
                    if let Some(compiled_fn) = self.jit.borrow_mut().compile(code) {
                        let precomputed = crate::jit::JitCompiler::precompute_with_names(code);
                        jit_ptr.set(compiled_fn as usize);
                        *jit_consts.borrow_mut() = precomputed;
                    }
                } else if jp != SENTINEL_FAILED {
                    // SAFETY: `jp` was just produced by `self.jit.borrow_mut().compile(code)`
                    // above (or on a prior call for the same `code`), which only ever emits
                    // machine code matching this exact `extern "C"` signature — the JIT
                    // codegen in jit.rs is the sole producer of values stored in `jit_ptr`.
                    let func_ptr: extern "C" fn(*const PyObjectRef, usize, *const PyObjectRef, *mut PyObjectRef) =
                        unsafe { std::mem::transmute(jp) };
                    let n = args.len().min(code.arg_count as usize);
                    let mut fast_locals: Vec<PyObjectRef> = Vec::with_capacity(n);
                    for i in 0..n {
                        fast_locals.push(args[i].clone());
                    }
                    let consts = jit_consts.borrow();
                    let mut result = PyObjectRef::None;
                    func_ptr(fast_locals.as_ptr(), fast_locals.len(), consts.as_ptr(), &mut result);
                    return Ok(result);
                }
            }

            // Try simple execution without Frame creation
            if defaults.is_empty() && keywords.is_empty() {
                if let Some(result) = Self::try_exec_simple(code, &args) {
                    return result;
                }
            }
            // A Python-level function call here recurses through actual
            // Rust call frames (`call_function` -> `execute()` ->
            // `execute_inner` -> `execute_instruction`'s `CALL` handling ->
            // `call_function` -> ...), with no equivalent of CPython's own
            // `sys.getrecursionlimit()` check anywhere — so unbounded
            // Python recursion (a plain accidental bug in user code, not
            // some contrived edge case) previously overflowed the REAL
            // native thread stack and hard-aborted the whole process
            // (`fatal runtime error: stack overflow`) instead of raising a
            // catchable `RecursionError`, exactly like real CPython does.
            // Confirmed general via the simplest possible repro (`def
            // f(n): return f(n+1)` called once) and via CPython's own
            // `test_isinstance.py`'s deliberate recursion-limit tests.
            // Reads `self.recursion_limit` (default matches real CPython's
            // `sys.getrecursionlimit()`, 1000 — see its own doc comment).
            // Made safe by `main.rs` running everything on a dedicated,
            // much larger-than-default stack sized with headroom to spare
            // even at the default limit.
            if self.frames.len() >= self.recursion_limit {
                return Err(PyError::recursion_error("maximum recursion depth exceeded"));
            }
            let func_globals = func_globals.clone();
            let defaults = defaults.clone();
            let code_rc = Rc::new(code.clone());
            let mut new_frame = self.acquire_frame(Rc::clone(&code_rc), func_globals, Rc::clone(&self.builtins), None);
            new_frame.closure = Box::new(closure.clone());
            let code = code;

            let npos = args.len();
            let named_params = code.arg_count;
            let fname = interner::lookup_str(code.name).to_string();

            fn format_missing_names(names: &[String]) -> String {
                match names.len() {
                    0 => String::new(),
                    1 => format!("'{}'", names[0]),
                    2 => format!("'{}' and '{}'", names[0], names[1]),
                    _ => {
                        let (last, rest) = names.split_last().unwrap();
                        let joined = rest.iter().map(|n| format!("'{}'", n)).collect::<Vec<_>>().join(", ");
                        format!("{}, and '{}'", joined, last)
                    }
                }
            }

            // Real Python raises `TypeError` immediately when more positional
            // arguments are given than the function accepts (and it has no
            // `*args` to absorb the excess) — this whole argument-binding
            // block had NO validation of any kind before this fix: too many
            // positional args were silently dropped, missing required args
            // were never detected (the function body would just hit
            // `LOAD_FAST unbound` chaos or read `None`), unexpected keyword
            // arguments were silently inserted as a throwaway local name,
            // and a keyword colliding with an already-positionally-filled
            // parameter silently overwrote it instead of raising. Found via
            // CPython's own `test_call.py`
            // (`TestErrorMessagesUseQualifiedName`/`CFunctionCallsErrorMessages`),
            // whose whole point is exercising exactly these error paths —
            // every single one of them was a real, silent correctness bug
            // affecting EVERY user-defined function call in the interpreter.
            if npos > named_params && code.vararg_name.is_none() {
                self.release_frame(new_frame);
                let num_defaults = code.num_defaults;
                let min_required = named_params.saturating_sub(num_defaults);
                let msg = if num_defaults == 0 {
                    format!("{}() takes {} positional argument{} but {} {} given",
                        fname, named_params, if named_params == 1 { "" } else { "s" },
                        npos, if npos == 1 { "was" } else { "were" })
                } else {
                    format!("{}() takes from {} to {} positional arguments but {} {} given",
                        fname, min_required, named_params, npos, if npos == 1 { "was" } else { "were" })
                };
                return Err(PyError::type_error(msg));
            }

            // Assign positional args to named parameters
            for i in 0..npos.min(named_params) {
                let name_clone = new_frame.code.varnames[i].to_string();
                new_frame.insert_local(&name_clone, args[i].clone());
                if i < new_frame.fast_locals.len() {
                    new_frame.fast_locals[i] = Some(args[i].clone());
                }
            }

            // Pack excess positional args into *args
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in named_params..npos {
                    extra.push(args[i].clone());
                }
                let vararg_val = py_tuple(extra);
                if let Some(idx) = new_frame.code.varnames.iter().position(|&n| crate::interner::intern_eq(n, vararg_name)) {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(vararg_val.clone());
                    }
                }
                new_frame.insert_local(&vararg_name, vararg_val);
            }

            // Apply defaults for missing positional params
            if npos < named_params {
                let num_defaults = code.num_defaults;
                // Parameters are split into two groups: those WITHOUT defaults (non-defaulted),
                // and those WITH defaults (defaulted). self (index 0) is never defaulted.
                // defaulted params start at index (named_params - num_defaults)
                let first_default = named_params - num_defaults;
                for i in npos..named_params {
                    if i >= first_default {
                        let default_idx = i - first_default;
                        let name_clone = new_frame.code.varnames[i].to_string();
                        let val = if default_idx < defaults.len() {
                            defaults[default_idx].clone()
                        } else {
                            py_none()
                        };
                        new_frame.insert_local(&name_clone, val.clone());
                        if i < new_frame.fast_locals.len() {
                            new_frame.fast_locals[i] = Some(val);
                        }
                    }
                }
            }

            // Handle **kwargs
            let kwonly_start = code.arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
            let positional_filled = npos.min(named_params);
            if let Some(kwarg_name) = &code.kwarg_name {
                let kw_dict = py_dict();
                for (key, value) in &keywords {
                    if let Some(idx) = formal_param_index(&new_frame.code.varnames, code.arg_count, code.kwonlyarg_count, kwonly_start, key) {
                        // A keyword targeting a formal parameter that ALREADY
                        // received a positional value — real Python's
                        // `TypeError: ...() got multiple values for argument
                        // '...'`, previously silently overwritten.
                        if idx < positional_filled {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!("{}() got multiple values for argument '{}'", fname, key)));
                        }
                        new_frame.insert_local(&key, value.clone());
                        if idx < new_frame.fast_locals.len() {
                            new_frame.fast_locals[idx] = Some(value.clone());
                        }
                    } else {
                        if let PyObject::Dict(ref mut dict) = &mut *kw_dict.borrow_mut() {
                            dict.set(py_str(key), value.clone())?;
                        }
                    }
                }
                if let Some(idx) = new_frame.code.varnames.iter().position(|n| crate::interner::lookup_str(*n) == kwarg_name.as_str()) {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(kw_dict.clone());
                    }
                }
                new_frame.insert_local(kwarg_name.as_str(), kw_dict);
            } else {
                // No **kwargs: keyword args must still bind to the matching
                // named parameter's FAST local slot (LOAD_FAST reads
                // fast_locals, not the insert_local name dict — missing this
                // meant `f(1, somekw=True)` left `somekw` as None in
                // fast_locals, raising "referenced before assignment" the
                // moment the function body read it), matching the
                // **kwargs branch above. A keyword matching no formal
                // parameter, or one that already got a positional value,
                // must raise `TypeError` — previously silently accepted as
                // either a no-op or a throwaway local-name insertion the
                // function body never referenced.
                for (key, value) in &keywords {
                    match formal_param_index(&new_frame.code.varnames, code.arg_count, code.kwonlyarg_count, kwonly_start, key) {
                        Some(idx) if idx < positional_filled => {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!("{}() got multiple values for argument '{}'", fname, key)));
                        }
                        Some(idx) => {
                            if idx < new_frame.fast_locals.len() {
                                new_frame.fast_locals[idx] = Some(value.clone());
                            }
                            new_frame.insert_local(&key, value.clone());
                        }
                        None => {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!("{}() got an unexpected keyword argument '{}'", fname, key)));
                        }
                    }
                }
            }

            // Apply defaults for still-unbound keyword-only params (CPython's
            // __kwdefaults__ equivalent) — must run after explicit keyword
            // binding above, since only truly-unbound kwonly slots should
            // get their default. Defaults for kwonly params live in
            // `defaults` right after the positional ones (see
            // CodeObject::kwonly_defaults_mask / MAKE_FUNCTION).
            if code.kwonlyarg_count > 0 {
                // A live `__kwdefaults__` dict set on the function (either the
                // default one or a REPLACEMENT — `f.__kwdefaults__ = {...}`
                // must affect subsequent calls, test_keywordonlyarg's
                // testKwDefaults) is the source of truth for kwonly defaults,
                // overriding the compiled-in ones.
                let live_kwdefaults: Option<Box<crate::object::PyDict>> = inner_f.dict.get("__kwdefaults__").and_then(|v| {
                    if let PyObject::Dict(d) = &*v.borrow() { Some(d.clone()) } else { None }
                });
                let kwonly_start = code.arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
                let mut kwdefault_idx = code.num_defaults;
                for (k, &has_default) in code.kwonly_defaults_mask.iter().enumerate() {
                    let idx = kwonly_start + k;
                    if idx >= new_frame.fast_locals.len() || new_frame.fast_locals[idx].is_some() {
                        continue;
                    }
                    let name_str = interner::lookup_str(new_frame.code.varnames[idx]).to_string();
                    let default_val = match &live_kwdefaults {
                        Some(d) => d.get(&py_str(&name_str)).ok().flatten(),
                        None => {
                            if !has_default { continue; }
                            let v = defaults.get(kwdefault_idx).cloned();
                            kwdefault_idx += 1;
                            v
                        }
                    };
                    if let Some(val) = default_val {
                        new_frame.insert_local(&name_str, val.clone());
                        new_frame.fast_locals[idx] = Some(val);
                    }
                }
            }

            // Any formal positional/keyword-only parameter still unbound at
            // this point has no value at all — real Python's `TypeError:
            // ...() missing N required positional/keyword-only argument(s):
            // '...'`, previously never checked.
            let missing_positional: Vec<String> = (0..named_params)
                .filter(|&i| i >= new_frame.fast_locals.len() || new_frame.fast_locals[i].is_none())
                .map(|i| interner::lookup_str(new_frame.code.varnames[i]).to_string())
                .collect();
            if !missing_positional.is_empty() {
                self.release_frame(new_frame);
                let n = missing_positional.len();
                return Err(PyError::type_error(format!("{}() missing {} required positional argument{}: {}",
                    fname, n, if n == 1 { "" } else { "s" }, format_missing_names(&missing_positional))));
            }
            let missing_kwonly: Vec<String> = (kwonly_start..kwonly_start + code.kwonlyarg_count)
                .filter(|&i| i >= new_frame.fast_locals.len() || new_frame.fast_locals[i].is_none())
                .map(|i| interner::lookup_str(new_frame.code.varnames[i]).to_string())
                .collect();
            if !missing_kwonly.is_empty() {
                self.release_frame(new_frame);
                let n = missing_kwonly.len();
                return Err(PyError::type_error(format!("{}() missing {} required keyword-only argument{}: {}",
                    fname, n, if n == 1 { "" } else { "s" }, format_missing_names(&missing_kwonly))));
            }

            self.frames.push(new_frame);
            let result = self.execute();
            if let Some(frame) = self.frames.pop() {
                self.release_frame(frame);
            }
            return result;
        }

        // `type` itself is special: calling it means `type(x)` (introspect)
        // or `type(name, bases, ns)` (metaclass-style construction), NOT
        // "instantiate the class named type" — must be checked by identity
        // before the generic Type-calling convention below, which would
        // otherwise build a plain Instance (wrong: `type(x)` must return a
        // real class/type object, not an instance of `type`).
        if self.builtins.get(&interner::intern("type")).map(|t| t.is(&callable)).unwrap_or(false) {
            if !keywords.is_empty() {
                return Err(PyError::type_error("type() takes no keyword arguments"));
            }
            if args.len() == 1 {
                // type(obj) -> obj's real type. For a plain instance/value
                // this is its class (unchanged from before); for a class
                // object itself, it's that class's *metaclass* — plain
                // `type` unless something built it with a custom one (see
                // METATYPE_KEY) — never the class itself, which is what
                // the old, metaclass-unaware fallback incorrectly returned.
                let is_type_obj = matches!(&*args[0].borrow(), PyObject::Type { .. });
                if is_type_obj {
                    let mt = crate::object::metatype_of(&args[0]);
                    return Ok(mt.unwrap_or_else(|| callable.clone()));
                }
                return crate::object::builtin_type_of(&args);
            }
            if args.len() == 3 {
                // type(name, bases, ns) -> dynamic class creation. Uses
                // `self` directly (not `builtin_type_of`'s own 3-arg path,
                // which goes through `with_vm_mut` — safe when called with
                // no VM already active, but we're already inside one here
                // and a second aliasing `&mut VirtualMachine` from within
                // this same call chain reliably segfaulted in testing).
                let bases_vec = match &*args[1].borrow() {
                    PyObject::Tuple(t) => t.clone(),
                    PyObject::None => vec![],
                    _ => vec![args[1].clone()],
                };
                let namespace_dict = crate::object::dict_arg_to_hashmap(&args[2], "type() third argument must be a dict")?;
                return self.default_build_class(args[0].str(), bases_vec, namespace_dict, vec![], None);
            }
            return crate::object::builtin_type_of(&args);
        }

        // A class built by a custom metaclass that itself defines
        // `__call__` (e.g. enum's `EnumType.__call__`, used for `Color(2)`
        // value-lookup instead of construction) must dispatch through that
        // `__call__` — real Python semantics: `SomeClass(...)` really means
        // `type(SomeClass).__call__(SomeClass, ...)`, and only the
        // *default* `type.__call__` behaves like "construct + __init__",
        // which is what the generic Type-calling convention below
        // hardwires. Checked by identity-of-behavior (does the metatype
        // have its OWN `__call__`, distinct from `object`'s absence of
        // one) rather than unconditionally, so plain classes with no
        // custom metaclass are completely unaffected.
        {
            let mt = crate::object::metatype_of(&callable);
            if let Some(mt) = mt {
                if let Some(call_fn) = crate::object::lookup_dunder_via_mro(&mt, "__call__") {
                    let unwrapped = if let PyObject::StaticMethod { func } = &*call_fn.borrow() { Some(func.clone()) } else { None };
                    let call_fn = unwrapped.unwrap_or(call_fn);
                    let mut call_args = vec![callable.clone()];
                    call_args.extend(args);
                    return self.call_function(call_fn, call_args, keywords);
                }
            }
        }

        // A real native value type (`int`, and eventually more — see
        // `NATIVE_VALUE_CTOR_KEY`'s doc comment) called DIRECTLY (`int(5)`,
        // as opposed to a user subclass like `class MyInt(int): ...`,
        // which still needs the Instance-building path below) dispatches
        // straight to its original native constructor and returns that
        // raw, UNWRAPPED result — never a `PyObject::Instance`. Checked
        // before `type_construct_info` so it takes priority over the
        // generic construction convention entirely.
        {
            let native_ctor = if let PyObject::Type { dict, .. } = &*callable.borrow() {
                dict.get_str(crate::object::NATIVE_VALUE_CTOR_KEY).cloned()
            } else {
                None
            };
            if let Some(ctor) = native_ctor {
                return self.call_function(ctor, args, keywords);
            }
        }

        let type_construct_info = if let PyObject::Type { dict, mro, .. } = &*callable.borrow() {
            let native_kind = dict.get_str(crate::object::NATIVE_BASE_MARKER).map(|v| v.str());
            let init_func = dict.get_str("__init__").cloned().or_else(|| {
                for base in mro.iter().skip(1) {
                    if let PyObject::Type { name: base_name, dict: base_dict, .. } = &*base.borrow() {
                        // Every class implicitly inherits from `object`,
                        // whose own __init__ is a universal no-op. For a
                        // class that also has a native base (e.g.
                        // `class SafeString(str, SafeData): ...`), that
                        // no-op would otherwise always be found first and
                        // preempt real native construction — skip it here
                        // so synthesize_native_init below gets a chance
                        // unless something more specific actually overrides
                        // __init__.
                        if native_kind.is_some() && base_name == "object" {
                            continue;
                        }
                        if let Some(val) = base_dict.get_str("__init__") {
                            return Some(val.clone());
                        }
                    }
                }
                None
            });
            Some((native_kind, init_func))
        } else {
            None
        };
        // The `callable.borrow()` above must be dropped (it already is, by
        // this point — the `if let` scrutinee's temporary ends with the
        // `if let` expression) before calling `__init__` below: `__init__`'s
        // body commonly references its own class by name (e.g. a
        // class-level counter like `Field.creation_counter += 1`, a
        // widespread real-world pattern, not specific to any one
        // library) — a STORE_ATTR on `callable` while this function still
        // held it borrowed here was a genuine double-borrow panic.
        if let Some((native_kind, init_func)) = type_construct_info {
            let mut instance_dict = AttrMap::new();
            if let Some(kind) = &native_kind {
                instance_dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), crate::object::make_native_backing(kind));
            }
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: callable.clone(),
                dict: instance_dict,
            });
            if init_func.is_none() {
                // No Python- or Rust-defined __init__ anywhere in the mro:
                // for a native-subclassing class (`class Foo(list): pass`),
                // that means the constructor call itself must behave like
                // list(iterable)/dict(...)/str(x).
                if let Some(kind) = &native_kind {
                    let native = crate::object::synthesize_native_init(kind, &args, &keywords)?;
                    if let PyObject::Instance { dict, .. } = &mut *instance.borrow_mut() {
                        dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), native);
                    }
                } else if crate::object::find_exception_base_name(&callable).is_some() {
                    // `class MyError(Exception): pass` (no explicit
                    // __init__) — real Python's `BaseException.__init__`
                    // always stores `self.args = args`, which is what
                    // `str(exc)`/`repr(exc)` and every uncaught-exception
                    // traceback print. Exception builtins (Exception,
                    // ValueError, ...) are `BuiltinFunction`s, not
                    // `PyObject::Type`s, so they never appear in `mro` and
                    // were completely invisible to this constructor logic —
                    // ANY user-defined exception subclass (an extremely
                    // common, foundational pattern) silently got no `args`
                    // at all, surfacing as "MyError: " (empty message) or
                    // "Exception: re-raise" (the internal dispatch tag)
                    // instead of the real message whenever it passed through
                    // a `with`/`finally` or propagated uncaught.
                    if let PyObject::Instance { dict, .. } = &mut *instance.borrow_mut() {
                        dict.insert_str("args", py_tuple(args.clone()));
                    }
                }
            }
            if let Some(init_func) = init_func {
                // Delegate to the real call_function instead of a hand-rolled
                // frame setup per callable kind — the latter (kept here for
                // a long time) never handled *args/**kwargs/default values at
                // all, silently binding missing parameters to None instead of
                // their real defaults and dropping every keyword argument
                // passed to the constructor. call_function already gets all
                // of that right for every callable variant (BuiltinFunction,
                // Function, Closure, ...).
                let mut init_args = vec![instance.clone()];
                init_args.extend(args);
                self.call_function(init_func, init_args, keywords)?;
            }
            return Ok(instance);
        }

        if let PyObject::BuildClass = &*callable.borrow() {
            if args.len() < 3 {
                return Err(PyError::type_error("__build_class__: need at least 3 arguments"));
            }
            let func = args[0].clone();
            let name = args[1].clone();
            let bases = args[2].clone();

            let name_str = match &*name.borrow() {
                PyObject::Str(s) => s.to_string(),
                _ => return Err(PyError::type_error("class name must be a string")),
            };

            // Bases and any explicit `metaclass=` are already fully
            // evaluated by the time __build_class__ is called (the
            // compiler evaluates func/name/*bases/**kwds before emitting
            // the CALL — see Stmt::ClassDef's compilation) — so the
            // effective metaclass can and must be determined BEFORE the
            // class body executes, not after: a metaclass's `__prepare__`
            // (e.g. enum's `_EnumDict`-returning one, needed for Django's
            // `ChoicesType` to see a real `_member_names` list) has to
            // exist as the body's own namespace target from the very
            // start, not spliced in afterward.
            let explicit_metaclass = keywords.iter()
                .find(|(k, _)| k == "metaclass")
                .map(|(_, v)| v.clone());

            let bases_vec = if matches!(&*bases.borrow(), PyObject::None) {
                vec![]
            } else if let PyObject::Tuple(t) = &*bases.borrow() {
                t.clone()
            } else {
                vec![bases.clone()]
            };
            // Classes without explicit bases implicitly inherit from object
            let bases_vec = if bases_vec.is_empty() {
                // Look up 'object' type from builtins
                let object_type = self.builtins.get(&interner::intern("object")).cloned()
                    .unwrap_or_else(|| {
                        // Fallback: create a minimal object type
                        let mut obj_dict: TypeDict = Default::default();
                        obj_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "__setattr__".to_string(),
                            func: |args| {
                                if args.len() < 3 { return Err(PyError::type_error("__setattr__ needs 3 args")); }
                                args[0].borrow_mut().set_attribute(&args[1].str(), args[2].clone())?;
                                Ok(py_none())
                            },
                        }));
                        PyObjectRef::new(PyObject::Type {
                            name: "object".to_string(),
                            dict: Box::new(obj_dict),
                            bases: vec![],
                            mro: vec![],
                        })
                    });
                vec![object_type]
            } else {
                bases_vec
            };

            // __init_subclass__ (and any custom metaclass __new__/__init__)
            // only ever see the non-`metaclass` keywords.
            let init_subclass_kwargs: Vec<(String, PyObjectRef)> = keywords.iter()
                .filter(|(k, _)| k != "metaclass")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            // Metaclass inheritance: without an explicit `metaclass=`, a
            // subclass of a class built by a custom metaclass must still be
            // built by that SAME metaclass (e.g. `class IntegerChoices(Choices,
            // IntEnum)` has no `metaclass=` of its own but must still use
            // `ChoicesType`, inherited from `Choices`) — matching CPython's
            // `_calculate_meta`. Real conflict resolution across multiple
            // unrelated custom metaclasses among the bases is out of scope
            // here (rare in practice); the first one found wins.
            let inherited_metaclass = bases_vec.iter().find_map(crate::object::metatype_of);
            let effective_metaclass = explicit_metaclass.or(inherited_metaclass);

            // If the effective metaclass defines `__prepare__`, call it now
            // (before the class body runs) to get the namespace object the
            // body's names should end up copied into. `__prepare__` is a
            // classmethod by convention — bind `mc` as its first arg
            // manually (mirroring StaticMethod/ClassMethod unwrapping used
            // elsewhere for `__new__`), since `lookup_dunder_via_mro`
            // itself does no descriptor binding.
            let prepared_namespace: Option<PyObjectRef> = if let Some(mc) = &effective_metaclass {
                crate::object::lookup_dunder_via_mro(mc, "__prepare__").and_then(|prep_fn| {
                    let unwrapped = match &*prep_fn.borrow() {
                        PyObject::ClassMethod { func } => func.clone(),
                        PyObject::StaticMethod { func } => func.clone(),
                        _ => prep_fn.clone(),
                    };
                    let is_classmethod = matches!(&*prep_fn.borrow(), PyObject::ClassMethod { .. });
                    let call_args = if is_classmethod {
                        vec![mc.clone(), name.clone(), bases.clone()]
                    } else {
                        vec![name.clone(), bases.clone()]
                    };
                    self.call_function(unwrapped, call_args, vec![]).ok()
                })
            } else {
                None
            };

            let namespace: Rc<RefCell<HashMap<StrId, PyObjectRef>>> = Rc::new(RefCell::new(HashMap::new()));
            let name_order = Rc::new(RefCell::new(Vec::new()));

            // Capture the calling frame's module_globals (or globals as fallback)
            // so that LOAD_NAME inside the class body can resolve module-level names.
            let caller_module_globals = if self.frames.len() >= 1 {
                let caller_frame = &self.frames[self.frames.len() - 1];
                caller_frame.module_globals.clone()
                    .or_else(|| Some(caller_frame.globals.clone()))
            } else {
                None
            };

            match &*func.borrow() {
                PyObject::Function(ref f) => {
            let code = &f.code;
            let closure = &f.closure;
                    let code = code.clone();
                    let closure = closure.clone();
                    let mut new_frame = self.acquire_frame(code, namespace.clone(), Rc::clone(&self.builtins), caller_module_globals);
                    new_frame.closure = Box::new(closure);
                    new_frame.name_order = Some(name_order.clone());
                    self.frames.push(new_frame);
                    // Must pop this frame unconditionally, including on
                    // error — `self.execute()?` used to return early on a
                    // class body raising mid-execution (e.g. a metaclass's
                    // `__init_subclass__`/descriptor processing failing),
                    // skipping the pop below and leaking the frame. Once
                    // leaked, `self.frames` never returns to the depth
                    // `handle_exception`'s `frame_floor` invariant assumes
                    // (exactly one frame per still-live `execute()` call),
                    // so a *later*, unrelated caught exception elsewhere
                    // finds the stack in a corrupted shape and blows up
                    // with "stack underflow (peek)" — confirmed via a
                    // frame_floor/frames.len() trace showing frames.len()
                    // staying flat across a frame_floor transition where it
                    // should have dropped by one.
                    let result = self.execute();
                    if let Some(frame) = self.frames.pop() {
                        self.release_frame(frame);
                    }
                    result?;
                }
                _ => return Err(PyError::type_error("class body must be a function")),
            }

            let namespace_dict: HashMap<String, PyObjectRef> = namespace.borrow().iter().map(|(k,v)| (interner::lookup_str(*k).to_string(), v.clone())).collect();
            let order = name_order.borrow().clone();

            // If `__prepare__` produced a namespace object, replay the body's
            // assignments into it in definition order via real `[key] =
            // value` subscript-assignment — this is what actually invokes
            // e.g. `_EnumDict.__setitem__` for each name, letting it build
            // up its own tracking (like `_member_names`) exactly as if the
            // class body had written directly into it. Doing this as a
            // replay *after* the body runs (rather than making STORE_NAME
            // itself dict-object-aware) keeps class body execution
            // unchanged for the overwhelming common case (no `__prepare__`)
            // at the cost of not supporting code that reads its own
            // not-yet-assigned names back out mid-body through the custom
            // namespace's own __getitem__ — not needed here.
            //
            // Deliberately does NOT use the free `py_setitem` function here
            // (unlike STORE_SUBSCR) — that dispatches a found `__setitem__`
            // via `call_bound_method`, which (a separate, pre-existing,
            // documented limitation) spins up a brand-new disposable
            // `VirtualMachine::new()` for the call. Since this code runs
            // during `install_source_defined_stdlib`'s enum bootstrap
            // (i.e. *during* a VM's own construction), that disposable VM's
            // construction re-runs the exact same enum bootstrap, which
            // hits this exact same replay again — genuine infinite
            // recursion (confirmed via gdb backtrace showing repeated
            // VirtualMachine::new() frames), not just wasted work. `self`
            // is already the one real, live VM here, so call its own
            // `call_function` directly instead.
            if let Some(prepared) = &prepared_namespace {
                let setitem_fn = if let PyObject::Instance { typ, .. } = &*prepared.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__setitem__")
                } else {
                    None
                };
                if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                    eprintln!("prepare-replay: name={} order={:?} has_setitem={}", name_str, order, setitem_fn.is_some());
                }
                for k in &order {
                    if let Some(v) = namespace_dict.get(k) {
                        if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                            eprintln!("  replaying key={} value={}", k, v.repr());
                        }
                        if let Some(f) = &setitem_fn {
                            self.call_function(f.clone(), vec![prepared.clone(), py_str(k), v.clone()], vec![])?;
                        } else if let Some(native) = crate::object::native_backing_of(prepared) {
                            if let PyObject::Dict(pd) = &mut *native.borrow_mut() {
                                pd.set(py_str(k), v.clone())?;
                            }
                        }
                    }
                }
                if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                    if let Some(native) = crate::object::native_backing_of(prepared) {
                        if let PyObject::Dict(pd) = &*native.borrow() {
                            eprintln!("  final native dict keys: {:?}", pd.keys().iter().map(|k| k.str()).collect::<Vec<_>>());
                        }
                    }
                }
            }

            if let Some(mc) = effective_metaclass {
                return self.build_class_with_metaclass(name_str, name.clone(), bases_vec, namespace_dict, order, mc, init_subclass_kwargs, prepared_namespace);
            }

            return self.default_build_class(name_str, bases_vec, namespace_dict, init_subclass_kwargs, None);
        }

        if let PyObject::Closure(c) = &*callable.borrow() {
            // Same "pack keywords into a trailing dict" convention as
            // `BuiltinFunction` just above — this early-return skipped it
            // entirely, so a `Closure`-implemented method called with
            // keyword arguments (real trigger: `namedtuple`'s own
            // `_replace(field=val)`) silently ran as if NO keywords were
            // passed at all (the override never took effect).
            if !keywords.is_empty() {
                let mut dict = crate::object::PyDict::new();
                for (k, v) in &keywords {
                    let _ = dict.set(crate::object::py_str(k), v.clone());
                }
                let mut new_args = args;
                new_args.push(crate::object::PyObjectRef::new(crate::object::PyObject::Dict(Box::new(dict))));
                return c(&new_args);
            }
            return c(&args);
        }

        let call_dunder = {
            let borrowed = callable.borrow();
            if let PyObject::Instance { typ, .. } = &*borrowed {
                crate::object::lookup_dunder_via_mro(typ, "__call__")
            } else {
                None
            }
        };
        if let Some(f) = call_dunder {
            // `callable` must not still be borrowed here — if `__call__`'s
            // own body mutates `self` (e.g. `self.hits += 1`, common for a
            // caching wrapper), STORE_ATTR's borrow_mut() on the very same
            // object would otherwise panic with a RefCell conflict.
            //
            // When the found `__call__` is ITSELF an `Instance` (not a
            // `Function`/`BuiltinFunction`), this recurses straight back
            // into `self.call_function` via plain Rust call-stack recursion
            // — no Python frame is ever pushed for this step, so it never
            // passes through the `PyObject::Function` arm's own
            // `self.frames.len() >= self.recursion_limit` check at all. A
            // cyclic `__call__` chain (real trigger: CPython's own
            // `test_descr.py`'s `test_recursive_call` — `A.__call__ = A()`,
            // then `A()()`) previously recursed forever and overflowed the
            // real native stack instead of raising a catchable
            // `RecursionError`. Guarded the same way as the other
            // "disposable-VM-shaped" native recursion gap this session (see
            // `NativeDispatchRecursionGuard`'s own doc comment).
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let mut call_args = vec![callable.clone()];
            call_args.extend(args.iter().cloned());
            return self.call_function(f, call_args, keywords);
        }

        Err(PyError::type_error(format!("'{}' object is not callable", type_name)))
    }

    fn synth_exception(typ: &str, error: &PyError) -> PyObjectRef {
        // MUST be `Mut` (via `PyObjectRef::new`), not `Imm` — this converts
        // EVERY native `PyError::TypeError`/`ValueError`/`ZeroDivisionError`/
        // etc. (i.e. almost every runtime error the interpreter itself
        // detects, as opposed to a user `raise SomeError(...)` statement,
        // which already goes through `exceptions_ctor.rs`'s correctly-`Mut`
        // constructor) into a Python-visible exception object. `STORE_ATTR`
        // unconditionally rejects setting ANY attribute on an `Imm`-wrapped
        // value (see its own doc comment) before ever reaching
        // `PyObject::Exception`'s own (already-correct, already-permissive)
        // `set_attribute` arm — so with the old `imm` constructor here,
        // `except TypeError as e: e.__traceback__ = tb` (an extremely
        // common idiom: `unittest`'s own `result.py`, `contextlib`'s
        // generator-context-manager `__exit__`, ...) raised `AttributeError`
        // for literally any exception synthesized this way. Confirmed via
        // CPython's own test suite: this exact bug surfaced across 24
        // DIFFERENT test files simultaneously (the single widest-reaching
        // bug found this whole session), all via this one shared root cause.
        PyObjectRef::new(PyObject::Exception {
            typ: typ.to_string(),
            args: vec![py_str(&error.message())],
            cause: None,
        })
    }

    /// The real CLASS object behind a raised exception instance — what
    /// `sys.exc_info()[0]` must be (see the `RAISE_VARARGS` call site that
    /// uses this). For a `class MyError(Exception): ...` instance this is
    /// its own `typ`; for the native `PyObject::Exception`/`ExceptionGroup`
    /// representations (a bare string type name, not a real class object)
    /// this looks the name up in `self.builtins` (where every builtin
    /// exception is registered as a `BuiltinFunction`/constructor) — falling
    /// back to a freshly-built placeholder `Type` sharing just the name if
    /// somehow not found there, rather than ever returning the instance
    /// itself (which is what caused `issubclass(exc_info()[0], ...)` to
    /// raise "arg 1 must be a class").
    fn exception_class_of(&self, exc: &PyObjectRef) -> PyObjectRef {
        let name = match &*exc.borrow() {
            PyObject::Instance { typ, .. } => return typ.clone(),
            PyObject::Exception { typ, .. } => typ.clone(),
            PyObject::ExceptionGroup { .. } => "ExceptionGroup".to_string(),
            other => other.type_name().to_string(),
        };
        if let Some(builtin) = self.builtins.get(&interner::intern(&name)) {
            return builtin.clone();
        }
        PyObjectRef::new(PyObject::Type {
            name,
            dict: Box::new(TypeDict::default()),
            bases: vec![],
            mro: vec![],
        })
    }

    /// The real exception OBJECT a `PyError` represents — shared by
    /// `handle_exception` (pushes it for the handler/CHECK_EXC_MATCH to see)
    /// and `execute()`/`throw_into_frame` (need the same real object, not a
    /// bare string, to populate `exc_value`/derive `exc_type` for
    /// `sys.exc_info()` — see those call sites' own comments for the exact
    /// bug this fixes).
    fn error_to_exc_obj(error: &PyError) -> PyObjectRef {
        match error {
            // Reuse the original PyObjectRef exactly as raised — preserves
            // object identity (needed for `except E as e: ... raise` to
            // compare `e` against the handler-bound exception, and for
            // CPython's own `exc is value` idiom as used by contextlib's
            // generator-based context managers), plus its real
            // args/__cause__/extra attrs, instead of rebuilding a throwaway
            // single-message copy.
            //
            // EXCEPT for one ad hoc shape: a generator's own
            // `__next__`/`send`/`throw` driver (`object.rs`'s Generator
            // match arm) signals "generator returned instead of yielding
            // again" as `PyError::Exception("StopIteration".into(),
            // return_value)` — `return_value` there is the generator's raw
            // return value (often `None`), NOT a real exception object (see
            // `is_stop_iteration_error`'s doc comment, which already knows
            // to check the message string for exactly this reason). Pushing
            // that raw value as-is meant a Python-level `except
            // StopIteration as exc:` clause could never recognize it
            // (CHECK_EXC_MATCH has nothing exception-shaped to classify),
            // breaking `contextlib.contextmanager`'s own `__exit__`, which
            // relies on exactly that to detect a generator finishing in
            // response to `.throw()`. Build a real `StopIteration` instance
            // instead, carrying the return value as its arg (matching real
            // CPython's `StopIteration(value)`).
            PyError::Exception(msg, exc) if msg == "StopIteration"
                && !matches!(&*exc.borrow(), PyObject::Exception { typ, .. } if typ == "StopIteration") =>
            {
                // Same `Mut`-not-`Imm` fix, same reason, as `synth_exception`
                // just above — a synthesized exception object must support
                // attribute assignment (`.__traceback__ = ...` etc.).
                PyObjectRef::new(PyObject::Exception {
                    typ: "StopIteration".to_string(),
                    args: vec![exc.clone()],
                    cause: None,
                })
            }
            PyError::Exception(_, exc) => exc.clone(),
            PyError::TypeError(_) => Self::synth_exception("TypeError", error),
            PyError::ValueError(_) => Self::synth_exception("ValueError", error),
            PyError::NameError(_) => Self::synth_exception("NameError", error),
            PyError::AttributeError(_) => Self::synth_exception("AttributeError", error),
            PyError::IndexError(_) => Self::synth_exception("IndexError", error),
            PyError::KeyError(_) => Self::synth_exception("KeyError", error),
            PyError::ZeroDivisionError(_) => Self::synth_exception("ZeroDivisionError", error),
            PyError::RuntimeError(_) => Self::synth_exception("RuntimeError", error),
            PyError::StopIteration => Self::synth_exception("StopIteration", error),
            PyError::ImportError(_) => Self::synth_exception("ImportError", error),
            PyError::RecursionError(_) => Self::synth_exception("RecursionError", error),
            // `PyError::OsError` (raised by essentially every file/OS
            // operation — `os.stat`/`open()`/`read()`/`write()`/etc. — for
            // any underlying `std::io::Error`) previously fell through to
            // the generic `_` catch-all below, synthesizing a bare
            // `Exception` instead of a real, catchable `OSError`. Broke the
            // extremely common `try: os.stat(path) except OSError:`
            // existence-check idiom (used throughout the real stdlib
            // itself — real trigger: vendoring `dbm/__init__.py`'s own
            // `whichdb()`, `except OSError:` around a missing-file
            // `os.stat()` call) and any other OS-error-handling code
            // anywhere in the ecosystem.
            PyError::OsError(_) => Self::synth_exception("OSError", error),
            _ => Self::synth_exception("Exception", error),
        }
    }

    fn handle_exception(&mut self, error: &PyError, frame_floor: usize) -> bool {
        // Only this execute_inner invocation's own frame may handle the
        // exception here — frames below `frame_floor` belong to an outer,
        // suspended execute() call and must never be touched from inside a
        // nested one (see the comment on `execute()` for why).
        for frame in self.frames[frame_floor..].iter_mut().rev() {
            while let Some(handler) = frame.exception_handlers.pop() {
                // For any handler (Except or Finally), restore stack and transfer control
                frame.stack.truncate(handler.stack_depth);
                frame.ip = handler.instr_addr;
                let exc_obj = Self::error_to_exc_obj(error);
                frame.push(exc_obj);
                // For Finally handlers, we always execute them.
                // For Except handlers, we also execute them — the code at the
                // handler address will check CHECK_EXC_MATCH to decide.
                // The key difference: after a Finally handler finishes, the
                // exception is re-raised via RERAISE (by the code the compiler
                // emits after the finally block). After an Except handler
                // finishes, there's no RERAISE — the exception was handled.
                return true;
            }
        }
        false
    }
}

/// C3 linearization for proper method resolution order (MRO).
///
/// Implements the C3 algorithm used by CPython since Python 2.3.
/// Merges the MROs of all bases together with the direct bases list.
/// Returns an error if a consistent MRO cannot be created.
fn c3_linearize(bases: &[PyObjectRef]) -> PyResult<Vec<PyObjectRef>> {
    if bases.is_empty() {
        return Ok(vec![]);
    }

    // Build the lists to merge:
    // For each base, get its linearized MRO (already computed since classes
    // are created in dependency order). If the base's MRO is empty (as with
    // built-in types whose MRO was never computed), treat it as just [base].
    // The C3 algorithm also includes the direct bases list as the last merge
    // list to enforce base ordering constraints.
    let mut lists: Vec<Vec<PyObjectRef>> = Vec::new();
    for base in bases {
        let base_mro = if let PyObject::Type { mro, .. } = &*base.borrow() {
            if mro.is_empty() {
                vec![base.clone()]
            } else {
                mro.clone()
            }
        } else {
            vec![base.clone()]
        };
        lists.push(base_mro);
    }
    // Add the direct bases list as the final merge constraint (C3 spec)
    lists.push(bases.to_vec());

    let mut result: Vec<PyObjectRef> = Vec::new();
    loop {
        // Collect non-empty lists
        let non_empty: Vec<&Vec<PyObjectRef>> = lists.iter().filter(|l| !l.is_empty()).collect();
        if non_empty.is_empty() {
            return Ok(result);
        }

        let mut found = false;
        'candidate: for list in &non_empty {
            let candidate = &list[0];

            // Check if candidate appears in the tail of any other non-empty list
            for other in &non_empty {
                if other.len() > 1 {
                    for item in &other[1..] {
                        if item.is(candidate) {
                            continue 'candidate;
                        }
                    }
                }
            }

            // Candidate is valid — add to result and remove from all heads
            result.push(candidate.clone());
            // Clone before mutable borrow to break borrow checker conflict
            let candidate_clone = candidate.clone();
            for list in &mut lists {
                if !list.is_empty() && list[0].is(&candidate_clone) {
                    list.remove(0);
                }
            }
            found = true;
            break;
        }

        if !found {
            return Err(PyError::type_error(
                "Cannot create a consistent method resolution order (MRO)"
            ));
        }
    }
}

impl VirtualMachine {
    /// Real implementation behind `type.__new__(metacls, name, bases,
    /// namespace, **kwds)`, called directly from `call_function` (see the
    /// special-case there) with genuine `&mut self` access — mirrors
    /// `crate::object::type_new_builtin`'s argument parsing exactly, but
    /// without needing `with_vm_mut`'s thread-local re-entrant VM lookup.
    fn type_new_impl(&mut self, args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.len() < 4 {
            return Err(PyError::type_error("type.__new__() takes at least 4 arguments (metacls, name, bases, namespace)"));
        }
        if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
            eprintln!("type_new_impl: args={:?}", args.iter().map(|a| format!("{}:{}", a.get_type_name(), a.repr())).collect::<Vec<_>>());
        }
        let metacls = args[0].clone();
        let name_str = args[1].str();
        let bases_vec = match &*args[2].borrow() {
            PyObject::Tuple(t) => t.clone(),
            PyObject::None => vec![],
            _ => vec![args[2].clone()],
        };
        let namespace_dict = crate::object::dict_arg_to_hashmap(&args[3], "type.__new__(): namespace must be a dict")?;
        let kwargs: Vec<(String, PyObjectRef)> = match args.get(4) {
            Some(d) => match &*d.borrow() {
                PyObject::Dict(d) => d.items().into_iter().map(|(k, v)| (k.str(), v)).collect(),
                _ => vec![],
            },
            None => vec![],
        };
        let is_bare_type = self.builtins.get(&interner::intern("type")).map(|t| t.is(&metacls)).unwrap_or(false);
        let metatype = if is_bare_type { None } else { Some(metacls) };
        self.default_build_class(name_str, bases_vec, namespace_dict, kwargs, metatype)
    }

    /// The plain (no custom metaclass) class-construction routine — this is
    /// the Rust equivalent of CPython's `type.__new__`: build the
    /// `PyObject::Type`, run C3 MRO linearization, apply `__set_name__` and
    /// `__init_subclass__`. Used directly for ordinary classes, and also
    /// exposed to Python code as `type.__new__` (see `type_new_builtin`
    /// below) so a custom metaclass's `__new__` can call
    /// `super().__new__(metacls, name, bases, namespace, **kwds)` and get
    /// this same construction — tagged with `metatype` so the result
    /// correctly reports which (customized) metaclass built it.
    pub(crate) fn default_build_class(
        &mut self,
        name_str: String,
        bases_vec: Vec<PyObjectRef>,
        mut namespace_dict: HashMap<String, PyObjectRef>,
        init_subclass_kwargs: Vec<(String, PyObjectRef)>,
        metatype: Option<PyObjectRef>,
    ) -> PyResult<PyObjectRef> {
        // Real CPython disallows subclassing `bool` outright (`TypeError:
        // type 'bool' is not an acceptable base type`) — unlike every other
        // migrated native type, `bool` is a real `PyObject::Type` (fixing
        // `type(True) is bool`) but deliberately NOT in
        // `is_recognized_native_base_name`, so it would otherwise fall
        // through to the generic `NATIVE_VALUE_CTOR_KEY`-based detection
        // arm just below and be silently treated as a valid native base
        // (constructing a nonsensical always-`False`-backed instance)
        // instead of raising. Checked by identity against the live `bool`
        // binding (not by name) so a shadowed/reassigned `bool` name
        // elsewhere doesn't false-positive.
        if let Some(bool_type) = self.builtins.get(&interner::intern("bool")) {
            for base in &bases_vec {
                if base.is(bool_type) {
                    return Err(PyError::type_error("type 'bool' is not an acceptable base type"));
                }
            }
        }

        // Detect `class Foo(list): ...` / `(dict)` / `(str)` / `(int)` —
        // either a direct native base, or inherited transitively through a
        // base that already carries the marker (propagated down so every
        // subclass's own dict has it, without needing to walk mro/bases
        // again at instantiation or dispatch time).
        for base in &bases_vec {
            let native_name = match &*base.borrow() {
                PyObject::BuiltinFunction { name, .. } if crate::object::is_recognized_native_base_name(name) => Some(name.clone()),
                // A native value type that's been migrated to a real
                // `PyObject::Type` (see `NATIVE_VALUE_CTOR_KEY`'s doc
                // comment — `int` as of this writing) is a second
                // recognized shape of "direct native base", alongside the
                // `BuiltinFunction` case above — `class MyInt(int): ...`
                // must keep working through this exact same
                // `NATIVE_BASE_MARKER`/native-backing machinery, unchanged.
                PyObject::Type { name, dict, .. } if dict.contains_key_str(crate::object::NATIVE_VALUE_CTOR_KEY) => Some(name.clone()),
                _ => crate::object::native_base_of_type(base),
            };
            if let Some(native_name) = native_name {
                namespace_dict.insert(crate::object::NATIVE_BASE_MARKER.to_string(), py_str(&native_name));
                break;
            }
        }

        if let Some(mt) = &metatype {
            namespace_dict.insert(crate::object::METATYPE_KEY.to_string(), mt.clone());
        }

        let class = PyObjectRef::new(PyObject::Type {
            name: name_str,
            dict: Box::new(str_map_to_typedict(namespace_dict.clone())),
            bases: bases_vec.clone(),
            mro: vec![],
        });

        let mut mro = vec![class.clone()];
        // C3 linearization for proper method resolution
        let linearization = c3_linearize(&bases_vec)?;
        mro.extend(linearization);
        if let PyObject::Type { mro: mro_field, .. } = &mut *class.borrow_mut() {
            *mro_field = mro;
        }
        crate::object::register_class(&class);

        // __set_name__ protocol: for each descriptor in the class dict that has __set_name__, call it
        for (attr_name, value) in namespace_dict.iter() {
            // Get __set_name__ from the TYPE (not the instance) to avoid double-binding
            let typ = match &*value.borrow() {
                PyObject::Instance { typ, .. } => Some(typ.clone()),
                _ => None,
            };
            let has_set_name = if let Some(t) = &typ {
                t.borrow().get_attribute("__set_name__").is_ok()
            } else {
                false
            };
            if has_set_name {
                if let Some(t) = typ {
                    let set_name_method = t.borrow().get_attribute("__set_name__").unwrap();
                    // Call with explicit self=value, then owner=class, name=attr_name
                    let _ = self.call_function(set_name_method, vec![value.clone(), class.clone(), py_str(attr_name)], vec![]);
                }
            }
        }

        // __init_subclass__ protocol: real CPython calls this EXACTLY ONCE
        // per class creation, via `super().__init_subclass__()` — which
        // walks the new class's own MRO (skipping the class itself) and
        // invokes the FIRST implementation found. This used to instead call
        // `get_attribute("__init_subclass__")` on every DIRECT base
        // independently, which — for any multiply-inherited class whose
        // several direct bases all resolve to the SAME shared ancestor
        // implementation (e.g. contextlib's `_GeneratorContextManager(
        // _GeneratorContextManagerBase, AbstractContextManager,
        // ContextDecorator)`, all sharing `object.__init_subclass__` — or,
        // more seriously, any two Django model mixins both resolving to
        // `AltersData.__init_subclass__`) called that ONE shared
        // implementation multiple times per class, redundantly at best and
        // — for an implementation with side effects, like Django's, which
        // lazily imports and re-walks `vars(cls)` — compounding into deep
        // reentrant recursion at worst (confirmed via a real repro: a
        // single `class MyModel(models.Model): pass` triggered 10+ nested
        // `AltersData.__init_subclass__` frames before failing).
        let self_mro = if let PyObject::Type { mro, .. } = &*class.borrow() { mro.clone() } else { vec![] };
        // Check each MRO entry's OWN direct dict (`get_str`), NOT the
        // recursive `get_attribute` (which re-walks THAT base's own MRO
        // from scratch and can resolve all the way down to `object`'s
        // shared no-op default on its own) — using `get_attribute` here
        // meant a multiply-inherited class whose FIRST base in MRO order
        // doesn't itself define `__init_subclass__` (e.g. a plain mixin
        // with no bases beyond implicit `object`) stopped at THAT base's
        // own inherited `object.__init_subclass__` default immediately,
        // never reaching a LATER base's real, meaningful override at all.
        // Real trigger: `class Combined(Mixin, unittest.TestCase): pass` —
        // `Mixin` (no explicit base) resolves `__init_subclass__` to
        // `object`'s default via its own separate MRO before `TestCase`'s
        // real override (which sets `_class_cleanups`, needed by
        // `TestCase.doClassCleanups`) is ever reached, silently skipping it
        // entirely. Checking each entry's OWN dict directly instead
        // correctly walks the single, already-flattened `self_mro` in
        // order — skipping bases with no direct definition — and still
        // calls the ultimate shared `object.__init_subclass__` default
        // exactly once if nothing else in the chain overrides it (this is
        // what the surrounding fix, described above, was for).
        let init_subclass = self_mro.iter().skip(1).find_map(|base| {
            if let PyObject::Type { dict, .. } = &*base.borrow() {
                dict.get_str("__init_subclass__").cloned()
            } else {
                None
            }
        });
        if let Some(init_subclass) = init_subclass {
            if std::env::var("RPY_DEBUG_INITSUBCLASS").is_ok() {
                let class_name = if let PyObject::Type { name, .. } = &*class.borrow() { name.clone() } else { "?".to_string() };
                eprintln!("INIT_SUBCLASS: class={}", class_name);
            }
            let _ = self.call_function(init_subclass, vec![class.clone()], init_subclass_kwargs.clone());
        }

        Ok(class)
    }

    /// Build a class via a custom metaclass (explicit `metaclass=` or one
    /// inherited from a base) — the general path real metaclasses (a
    /// user-defined class subclassing `type`, e.g. an enum's `EnumType`)
    /// need: look up `__new__` on the metaclass's own MRO and call it with
    /// the real CPython `__new__(metacls, name, bases, namespace, **kwds)`
    /// convention, falling back to the plain `default_build_class` (tagged
    /// with this metaclass) if the metaclass doesn't override `__new__`
    /// anywhere short of plain `type`. Also calls `__init__` on the
    /// metaclass afterward, if defined, mirroring normal instantiation.
    fn build_class_with_metaclass(
        &mut self,
        name_str: String,
        name_obj: PyObjectRef,
        bases_vec: Vec<PyObjectRef>,
        namespace_dict: HashMap<String, PyObjectRef>,
        order: Vec<String>,
        metaclass: PyObjectRef,
        init_subclass_kwargs: Vec<(String, PyObjectRef)>,
        prepared_namespace: Option<PyObjectRef>,
    ) -> PyResult<PyObjectRef> {
        // Ordered PyDict — class/metaclass namespace order is user-visible
        // (e.g. an enum's member definition order) and plain HashMap
        // iteration doesn't preserve it, so lay `order` down first. If the
        // metaclass's own `__prepare__` already produced a (now-populated)
        // namespace object — e.g. enum's `_EnumDict`, which tracked member
        // names via its own `__setitem__` as each entry was replayed into
        // it — use that object itself instead of building a fresh plain
        // dict, so extra attributes/state it accumulated (like
        // `_member_names`) survive into what the metaclass's `__new__`
        // receives.
        let namespace_py_dict = if let Some(prepared) = prepared_namespace {
            prepared
        } else {
            let mut pd = PyDict::new();
            for k in &order {
                if let Some(v) = namespace_dict.get(k) {
                    pd.set(py_str(k), v.clone())?;
                }
            }
            for (k, v) in &namespace_dict {
                if !order.contains(k) {
                    pd.set(py_str(k), v.clone())?;
                }
            }
            PyObjectRef::new(PyObject::Dict(Box::new(pd)))
        };
        let bases_tuple = PyObjectRef::imm(PyObject::Tuple(bases_vec.clone()));

        // `__new__` may be wrapped in StaticMethod (as `type.__new__` is,
        // and as a user metaclass's own `__new__` implicitly is too, since
        // `__new__` is always an implicit staticmethod in real Python) —
        // unwrap before calling, same as Type's own get_attribute does for
        // plain class-attribute access.
        let new_fn = crate::object::lookup_dunder_via_mro(&metaclass, "__new__").map(|v| {
            let unwrapped = if let PyObject::StaticMethod { func } = &*v.borrow() { Some(func.clone()) } else { None };
            unwrapped.unwrap_or(v)
        });

        let cls = if let Some(new_fn) = new_fn {
            if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                eprintln!("build_class_with_metaclass: name={} metaclass={} new_fn={}", name_str, metaclass.repr(), new_fn.repr());
            }
            self.call_function(
                new_fn,
                vec![metaclass.clone(), name_obj.clone(), bases_tuple.clone(), namespace_py_dict.clone()],
                init_subclass_kwargs.clone(),
            )?
        } else {
            // No __new__ anywhere in the metaclass's own mro (shouldn't
            // normally happen once `type` is registered with one) — fall
            // back to plain construction, still tagged with this metaclass.
            self.default_build_class(name_str, bases_vec, namespace_dict, init_subclass_kwargs.clone(), Some(metaclass.clone()))?
        };

        if let Some(init_fn) = crate::object::lookup_dunder_via_mro(&metaclass, "__init__") {
            let unwrapped = if let PyObject::StaticMethod { func } = &*init_fn.borrow() { Some(func.clone()) } else { None };
            let init_fn = unwrapped.unwrap_or(init_fn);
            let _ = self.call_function(init_fn, vec![cls.clone(), name_obj, bases_tuple, namespace_py_dict], init_subclass_kwargs);
        }

        Ok(cls)
    }

    /// Call __next__ on a user-class iterator. Used by FOR_ITER for Instance types.
    fn for_iter_next(&mut self, iter_val: PyObjectRef, jump_offset: u32) -> PyResult<Option<PyObjectRef>> {
        use crate::object::ObjectAccess;
        let next_method = iter_val.borrow().get_attribute("__next__");
        if let Ok(func) = next_method {
            match self.call_function(func, vec![], vec![]) {
                Ok(val) => {
                    self.frames.last_mut().unwrap().push(iter_val);
                    self.frames.last_mut().unwrap().push(val);
                    Ok(None)
                }
                Err(e) if crate::object::is_stop_iteration_error(&e) => {
                    self.frames.last_mut().unwrap().ip = jump_offset as usize;
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        } else {
            self.frames.last_mut().unwrap().ip = jump_offset as usize;
            Ok(None)
        }
    }
}

/// Checks if `child_type` is a subclass of (or the same type as) `parent_type`.
/// Defines the standard Python exception type hierarchy for the simplified
/// string-based type system used by this RustPython implementation.
/// Each exception type maps to its parent; walking up the chain determines
/// subclass relationships. Unknown types default to children of Exception.
/// Resolves an `except` clause's type expression against a raised
/// exception's type name — handling the common `except (A, B):` tuple form
/// (matches if ANY member matches), not just a single bare type/name.
fn exc_type_matches(expected: &PyObjectRef, exc_type_name: &str) -> PyResult<bool> {
    match &*expected.borrow() {
        // Same gap, same fix, as `builtin_issubclass`'s matching `Str` arm:
        // a bare string legitimately reaches here only via the internal
        // `WITH_EXIT` exc_type-name fallback (always a recognized
        // builtin/module exception name) — a real `except "spam":` clause
        // catching an arbitrary string must raise `TypeError: catching
        // classes that do not inherit from BaseException is not allowed`
        // instead of silently doing a by-name comparison (confirmed via
        // CPython's own `test_baseexception.py`'s
        // `UsageTests.test_catch_string`).
        PyObject::Str(s) if is_builtin_exception_class_name(s) => Ok(is_exception_subclass(exc_type_name, s)),
        PyObject::Type { name, bases, .. } => {
            // Real Python raises `TypeError: catching classes that do not
            // inherit from BaseException is not allowed` the moment an
            // `except SomeClass:` clause is evaluated against a class that
            // isn't actually an exception class — previously this arm just
            // did a by-name comparison unconditionally, so `except
            // NonBaseException:` (a plain `class NonBaseException(object):
            // pass`) silently never matched instead of raising (confirmed
            // via CPython's own `test_baseexception.py`'s
            // `UsageTests.test_catch_non_BaseException`). A user-defined
            // class's ancestry includes a real exception base iff
            // `find_exception_base_name` finds a `BuiltinFunction`-shaped
            // ancestor in its `bases`/`mro` (every built-in/module exception
            // "class" in this codebase is represented that way) — `None`
            // means it doesn't inherit from `BaseException` at all.
            //
            // EXCEPT: some native ad-hoc "exception classes" built directly
            // in Rust (e.g. `subprocess.CalledProcessError` in `net.rs`) are
            // deliberately constructed with `bases: vec![]` — there's no
            // native `Exception` `Type` to list as a real base (builtin
            // exceptions are `BuiltinFunction`s, not `Type`s), a known,
            // documented simplification, NOT a sign the class isn't really
            // an exception. The distinguishing signal: any class built via
            // REAL Python `class X(...): ...` syntax always ends up with a
            // non-empty `bases` — even `class Foo: pass` with no explicit
            // parent gets `(object,)` inserted (confirmed: `Foo.__bases__ ==
            // (object,)`) — so completely empty `bases` can only mean one of
            // these native marker types, never genuine user code. Skipping
            // the ancestry check for those (falling through to the same
            // name-based comparison as before) restores `except
            // subprocess.CalledProcessError:` matching without reopening the
            // `NonBaseException`/`test_catch_non_BaseException` gap this
            // whole check exists to close — confirmed regression via the
            // very next sweep after landing that check, in
            // `test_graphlib.py`'s `test_static_order_does_not_change_with_
            // the_hash_seed` (uses `script_helper.assert_python_ok`, which
            // catches `subprocess.CalledProcessError`).
            if !bases.is_empty() && crate::object::find_exception_base_name(expected).is_none() {
                return Err(PyError::type_error("catching classes that do not inherit from BaseException is not allowed"));
            }
            Ok(is_exception_subclass(exc_type_name, name))
        }
        PyObject::BuiltinFunction { name, .. } => Ok(is_exception_subclass(exc_type_name, name)),
        PyObject::Tuple(items) | PyObject::List(items) => {
            for item in items {
                if exc_type_matches(item, exc_type_name)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Err(PyError::type_error("catching classes that do not inherit from BaseException is not allowed")),
    }
}

pub(crate) fn is_exception_subclass(child_type: &str, parent_type: &str) -> bool {
    if child_type == parent_type {
        return true;
    }
    // Map each exception type to its direct parent in the hierarchy.
    // BaseException is the root — it has no parent.
    let parent: Option<&str> = match child_type {
        "BaseException" => None,
        "Exception" | "SystemExit" | "KeyboardInterrupt" | "GeneratorExit" |
        "BaseExceptionGroup" => Some("BaseException"),
        // Sub-hierarchy parents (intermediate nodes in the tree)
        "ArithmeticError" | "LookupError" | "ImportError" | "RuntimeError" |
        "Warning" | "OSError" | "ValueError" => Some("Exception"),
        "CycleError" => Some("ValueError"),
        "DecimalException" => Some("ArithmeticError"),
        "InvalidOperation" | "DivisionByZero" | "Inexact" | "Rounded" |
        "Clamped" | "Overflow" | "Underflow" | "FloatOperation" => Some("DecimalException"),
        "PickleError" => Some("Exception"),
        "PicklingError" | "UnpicklingError" => Some("PickleError"),
        // ExceptionGroup inherits from Exception
        "ExceptionGroup" => Some("Exception"),
        // Sub-hierarchy children — must come before leaves to not be shadowed
        // Children of ArithmeticError
        "FloatingPointError" | "OverflowError" | "ZeroDivisionError" => Some("ArithmeticError"),
        // Children of LookupError
        "IndexError" | "KeyError" => Some("LookupError"),
        // Children of OSError
        "EnvironmentError" | "IOError" => Some("OSError"),
        "FileNotFoundError" | "PermissionError" | "NotADirectoryError" |
        "IsADirectoryError" | "FileExistsError" => Some("OSError"),
        "ConnectionError" | "BrokenPipeError" | "ConnectionAbortedError" |
        "ConnectionRefusedError" | "ConnectionResetError" => Some("OSError"),
        "BlockingIOError" | "ChildProcessError" | "InterruptedError" |
        "ProcessLookupError" | "TimeoutError" => Some("OSError"),
        // Children of RuntimeError
        "NotImplementedError" | "RecursionError" => Some("RuntimeError"),
        // Children of ImportError
        "ModuleNotFoundError" => Some("ImportError"),
        // Children of NameError
        "UnboundLocalError" => Some("NameError"),
        // Children of SyntaxError — both were previously falling through to
        // the generic `_ => Some("Exception")` catch-all (real CPython:
        // `IndentationError(SyntaxError)`, `TabError(IndentationError)`).
        "IndentationError" => Some("SyntaxError"),
        "TabError" => Some("IndentationError"),
        // Children of ValueError
        "UnicodeError" | "UnicodeEncodeError" | "UnicodeDecodeError" |
        "UnicodeTranslateError" => Some("ValueError"),
        // `binascii.Error` — real CPython subclasses `ValueError` (checked
        // via `issubclass(binascii.Error, ValueError)`), found missing while
        // fixing `base64.b32decode`'s error validation (its own tests do
        // `assertRaises(binascii.Error, ...)`, which needs this ancestry to
        // also accept a plain `ValueError` the same way `assertRaises`
        // matching real CPython would).
        "Error" => Some("ValueError"),
        // Children of Warning
        "UserWarning" | "DeprecationWarning" | "PendingDeprecationWarning" |
        "SyntaxWarning" | "RuntimeWarning" | "FutureWarning" |
        "ImportWarning" | "UnicodeWarning" | "BytesWarning" |
        "ResourceWarning" => Some("Warning"),
        // Leaf exception types — directly under Exception, no subclasses
        "TypeError" | "NameError" | "AttributeError" |
        "StopIteration" | "StopAsyncIteration" | "AssertionError" |
        "BufferError" | "EOFError" | "MatchError" | "ReferenceError" |
        "MemoryError" => Some("Exception"),
        // Unknown types default to Exception (users can define subclasses)
        _ => Some("Exception"),
    };
    match parent {
        Some(p) => {
            if p == parent_type {
                true
            } else {
                is_exception_subclass(p, parent_type)
            }
        }
        None => false,
    }
}

/// Implements Python's Format Specification Mini-Language.
///
/// Parses a format spec string in the form:
/// `[[fill]align][sign][#][0][width][grouping_option][.precision][type]`
/// and applies the formatting to the given value.
///
/// See: https://docs.python.org/3/library/string.html#formatspec
pub fn format_with_spec(val: &PyObjectRef, spec_str: &str) -> PyResult<String> {
    if spec_str.is_empty() {
        return Ok(val.str());
    }

    let chars: Vec<char> = spec_str.chars().collect();
    let len = chars.len();
    let mut idx = 0;

    // --- parse [[fill]align] ---
    let fill_char;
    let align;
    if idx + 1 < len && matches!(chars[idx + 1], '<' | '>' | '^' | '=') {
        fill_char = chars[idx];
        align = chars[idx + 1];
        idx += 2;
    } else if idx < len && matches!(chars[idx], '<' | '>' | '^' | '=') {
        fill_char = ' ';
        align = chars[idx];
        idx += 1;
    } else {
        fill_char = ' ';
        align = '>';
    }

    // --- parse [sign] ---
    let sign = if idx < len && matches!(chars[idx], '+' | '-' | ' ') {
        let s = chars[idx];
        idx += 1;
        s
    } else {
        '-'  // default: show sign only for negatives
    };

    // --- parse [#] ---
    let alternate = if idx < len && chars[idx] == '#' { idx += 1; true } else { false };

    // --- parse [0] (zero-pad flag) ---
    // Note: '0' after width means just a digit, not zero-pad.
    // But Python's spec has '0' right after the sign/# before width.
    // We check if the next char is '0' AND is followed by a digit (width).
    let mut zero_pad = false;
    if idx < len && chars[idx] == '0' {
        // If '0' is followed by a digit or end, it's the start of width with zero-padding
        zero_pad = true;
        if idx + 1 < len && chars[idx + 1].is_ascii_digit() {
            idx += 1; // consume the '0' — it becomes part of width
        } else {
            idx += 1; // just '0' with no width
        }
    }

    // --- parse [width] ---
    let width: Option<usize> = {
        let start = idx;
        while idx < len && chars[idx].is_ascii_digit() { idx += 1; }
        if idx > start {
            // A format spec width this large (more digits than fit in a
            // `usize`) is nonsensical for any real display — real CPython
            // raises `ValueError` for absurd widths/precisions (deliberately
            // tested: CPython's own `test_format.py::test_precision` builds
            // a `.%sf % (sys.maxsize + 1)` spec specifically to check this)
            // rather than crashing. Bare `.unwrap()` here panicked the whole
            // process on `ParseIntError` instead.
            let w = chars[start..idx].iter().collect::<String>().parse::<usize>()
                .map_err(|_| PyError::value_error("Format specifier width too large"))?;
            // Unlike the overflow case above, a width of e.g. `sys.maxsize +
            // 1` (2**63) parses into a `usize` just fine — but actually
            // padding a string out to that length tries to allocate an
            // astronomical buffer (`apply_padding`'s `fill.repeat(w -
            // s.len())`), aborting the whole process with "memory
            // allocation of N bytes failed" instead of raising a catchable
            // `ValueError`. Same real trigger as the precision cap below
            // (`test_format.py::test_format_huge_width`, `.../huge_item_
            // number`) — capped at the same threshold for consistency.
            if w > 1000 {
                return Err(PyError::value_error("Format specifier width too large"));
            }
            Some(w)
        } else {
            None
        }
    };

    // Go back if we consumed '0' but it wasn't really zero-pad (no width follows)
    if zero_pad && width.is_none() {
        // The '0' was just a literal zero in a width-less spec — not valid, treat as no-op
        zero_pad = false;
    }

    // --- parse grouping option [,|_] ---
    if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
        idx += 1;
    }

    // --- parse [.precision] ---
    let precision: Option<usize> = if idx < len && chars[idx] == '.' {
        idx += 1;
        let start = idx;
        while idx < len && chars[idx].is_ascii_digit() { idx += 1; }
        if idx > start {
            // See the matching `width` comment above — same overflow-panic
            // fix, same real trigger (`test_format.py::test_precision`'s
            // `.%sf % (sys.maxsize + 1)`).
            let p = chars[start..idx].iter().collect::<String>().parse::<usize>()
                .map_err(|_| PyError::value_error("Format specifier precision too large"))?;
            // A precision this large parses fine as a `usize` (e.g.
            // `sys.maxsize + 1` == 2**63, well within range) but Rust's own
            // `format!("{:.prec$}", ...)` panics with "argument out of
            // range" trying to render it (asking for ~9*10^18 decimal
            // digits of a float is obviously never actually intended) —
            // confirmed via CPython's own `test_format.py::test_precision`,
            // which deliberately builds `.%sf % (sys.maxsize + 1)`
            // expecting a catchable `ValueError`, not a process crash.
            // 1000 decimal digits is already far beyond any real
            // formatting need (a `f64`'s own precision exhausts after ~17
            // significant digits) but comfortably below wherever Rust's
            // internal limit actually sits.
            if p > 1000 {
                return Err(PyError::value_error("precision too big"));
            }
            Some(p)
        } else {
            Some(0) // '.' with no digits means precision 0
        }
    } else {
        None
    };

    // --- parse [type] ---
    let fmt_type = if idx < len { Some(chars[idx]) } else { None };

    // Determine value type
    let val_borrowed = val.borrow();
    let is_int = matches!(&*val_borrowed, PyObject::Int(_) | PyObject::Bool(_));
    let is_float = matches!(&*val_borrowed, PyObject::Float(_));

    // Generate the formatted value based on type
    let base = match (fmt_type, is_int, is_float) {
        // Integer: decimal (default or 'd')
        (None, true, _) | (Some('d'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                let s = format_int_with_sign(i, sign, precision);
                s
            } else if let PyObject::Bool(b) = &*val_borrowed {
                format!("{}", if *b { 1i64 } else { 0i64 })
            } else {
                val.str()
            }
        }
        // Integer: hex lowercase
        (Some('x'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                if alternate { format!("0x{:x}", i) } else { format!("{:x}", i) }
            } else { val.str() }
        }
        // Integer: hex uppercase
        (Some('X'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                if alternate { format!("0X{:X}", i) } else { format!("{:X}", i) }
            } else { val.str() }
        }
        // Integer: binary
        (Some('b'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                if alternate { format!("0b{:b}", i) } else { format!("{:b}", i) }
            } else { val.str() }
        }
        // Integer: octal
        (Some('o'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                if alternate { format!("0o{:o}", i) } else { format!("{:o}", i) }
            } else { val.str() }
        }
        // Integer: character
        (Some('c'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                if let Some(n) = i.to_u32() {
                    if let Some(c) = char::from_u32(n) {
                        c.to_string()
                    } else {
                        return Err(PyError::value_error("chr() arg not in range(0x110000)"));
                    }
                } else {
                    return Err(PyError::value_error("chr() arg not in range(0x110000)"));
                }
            } else {
                return Err(PyError::type_error("integer argument expected, got float"));
            }
        }

        // Float: default (no type) — use str() for compat
        (None, _, true) => val.str(),
        // Float: fixed-point
        (Some('f'), _, true) | (Some('F'), _, true) => {
            if let PyObject::Float(f) = &*val_borrowed {
                let s = format_float_with_sign(*f, sign, precision);
                s
            } else { val.str() }
        }
        // Float: scientific lowercase
        (Some('e'), _, true) => {
            if let PyObject::Float(f) = &*val_borrowed {
                let s = match precision {
                    Some(p) => format!("{:.prec$e}", f, prec = p),
                    None => format!("{:e}", f),
                };
                // Apply sign
                apply_sign(&s, *f, sign)
            } else { val.str() }
        }
        // Float: scientific uppercase
        (Some('E'), _, true) => {
            if let PyObject::Float(f) = &*val_borrowed {
                let s = match precision {
                    Some(p) => format!("{:.prec$E}", f, prec = p),
                    None => format!("{:E}", f),
                };
                apply_sign(&s, *f, sign)
            } else { val.str() }
        }
        // Float: general lowercase
        (Some('g'), _, true) => {
            if let PyObject::Float(f) = &*val_borrowed {
                let s = match precision {
                    Some(p) => format!("{:.prec$}", f, prec = p),
                    None => format!("{}", f),
                };
                apply_sign(&s, *f, sign)
            } else { val.str() }
        }
        // Float: general uppercase
        (Some('G'), _, true) => {
            if let PyObject::Float(f) = &*val_borrowed {
                let s = match precision {
                    Some(p) => format!("{:.prec$}", f, prec = p).to_uppercase(),
                    None => format!("{}", f).to_uppercase(),
                };
                apply_sign(&s, *f, sign)
            } else { val.str() }
        }
        // Float: percentage
        (Some('%'), _, true) => {
            if let PyObject::Float(f) = &*val_borrowed {
                let pct = f * 100.0;
                let s = match precision {
                    Some(p) => format!("{:.prec$}", pct, prec = p),
                    None => format!("{}", pct),
                };
                format!("{}%", s)
            } else { val.str() }
        }

        // Default for string or any other type: str() representation
        _ => val.str(),
    };

    // Apply zero-padding (fill='0', align='=' for numbers)
    let base = if zero_pad {
        let effective_align = '=';
        apply_padding(&base, width, effective_align, '0', true)
    } else {
        base
    };

    // Apply final width and alignment
    let result = apply_padding(&base, width, align, fill_char, false);

    Ok(result)
}

/// Apply '+'/' '/'-' sign prefix. If `sign` is '-', only negative numbers get a '-'.
/// If `sign` is '+', positive numbers get '+', negative get '-'.
/// If `sign` is ' ', positive numbers get ' ', negative get '-'.
fn apply_sign(s: &str, val: f64, sign: char) -> String {
    if val < 0.0 {
        // Negative — Rust format already includes '-'
        format!("-{}", &s.trim_start_matches('-'))
    } else {
        match sign {
            '+' => format!("+{}", s),
            ' ' => format!(" {}", s),
            '-' => s.to_string(),
            _ => s.to_string(),
        }
    }
}

/// Format a BigInt with sign handling for Python format spec.
fn format_int_with_sign(i: &BigInt, sign: char, precision: Option<usize>) -> String {
    let s = if i.sign() == num_bigint::Sign::Minus {
        // Remove negative sign from BigInt's display, we'll add it back
        let abs_s = format!("{}", i).trim_start_matches('-').to_string();
        let s = match precision {
            Some(p) if p > abs_s.len() => {
                let zeros = "0".repeat(p - abs_s.len());
                format!("{}{}", zeros, abs_s)
            }
            _ => abs_s,
        };
        format!("-{}", s)
    } else {
        let abs_s = format!("{}", i);
        let s = match precision {
            Some(p) if p > abs_s.len() => {
                let zeros = "0".repeat(p - abs_s.len());
                format!("{}{}", zeros, abs_s)
            }
            _ => abs_s,
        };
        match sign {
            '+' => format!("+{}", s),
            ' ' => format!(" {}", s),
            '-' => s,
            _ => s,
        }
    };
    s
}

/// Format a float with sign and precision.
fn format_float_with_sign(val: f64, sign: char, precision: Option<usize>) -> String {
    let s = match precision {
        Some(p) => format!("{:.prec$}", val, prec = p),
        None => format!("{}", val),
    };
    apply_sign(&s, val, sign)
}

/// Apply padding/alignment to a base string.
fn apply_padding(s: &str, width: Option<usize>, align: char, fill: char, zero_mode: bool) -> String {
    let w = match width {
        Some(w) => w,
        None => return s.to_string(),
    };
    if s.len() >= w {
        return s.to_string();
    }
    let padding = w - s.len();
    let pad_str: String = fill.to_string().repeat(padding);

    match align {
        '<' => format!("{}{}", s, pad_str),
        '>' => format!("{}{}", pad_str, s),
        '^' => {
            let left = padding / 2;
            let right = padding - left;
            format!("{}{}{}", fill.to_string().repeat(left), s, fill.to_string().repeat(right))
        }
        '=' => {
            // Insert padding after sign (if any) but before digits
            if zero_mode {
                // For zero-pad mode, just left-pad
                format!("{}{}", pad_str, s)
            } else {
                // For '=' alignment with custom fill, insert after any leading sign
                if s.starts_with('+') || s.starts_with('-') || s.starts_with(' ') {
                    let (sign_byte, rest) = s.split_at(1);
                    format!("{}{}{}", sign_byte, pad_str, rest)
                } else {
                    format!("{}{}", pad_str, s)
                }
            }
        }
        _ => format!("{}{}", pad_str, s), // default right-align
    }
}
