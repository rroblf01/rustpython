use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use smallvec::SmallVec;
use crate::bytecode::*;
use crate::interner::{self, InternedMap};
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
    pub globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
    pub builtins: Rc<HashMap<String, PyObjectRef>>,
    pub stack: SmallVec<[PyObjectRef; 8]>,
    pub ip: usize,
    pub base_sp: usize,
    pub exception_handlers: Vec<ExceptionHandler>,
    pub return_value: Option<PyResult<PyObjectRef>>,
    pub closure: Vec<PyObjectRef>,
    /// Active exception for re-raise. Set by PUSH_EXC_INFO, consumed by RERAISE.
    /// This is separate from the value stack so that POP_EXCEPT (which pops the
    /// exception from the value stack) does not break RERAISE in try/finally blocks.
    pub active_exception: Option<PyObjectRef>,
    /// Inline attribute cache — caches LOAD_ATTR results per instruction offset.
    /// Cleared when the frame is created; populated on first attribute access.
    pub attr_cache: Vec<Option<(u64, PyObjectRef)>>,  // (type_version_tag, cached_value)
    /// Inline global cache — caches LOAD_GLOBAL results per instruction offset.
    pub global_cache: Vec<Option<PyObjectRef>>,
    /// Virtual registers for register-based bytecode execution.
    /// 256 virtual registers (u8 index) — no stack needed for most ops.
    pub registers: Vec<Option<PyObjectRef>>,
    /// Optional reference to the enclosing module's globals.
    /// Used by class bodies to resolve LOAD_NAME against module-level names
    /// and by MAKE_FUNCTION to set __module__ on created functions.
    pub module_globals: Option<Rc<RefCell<HashMap<String, PyObjectRef>>>>,
}

#[derive(Clone)]
pub struct ExceptionHandler {
    pub instr_addr: usize,
    pub stack_depth: usize,
}

impl Frame {
    pub fn new(
        code: Rc<CodeObject>,
        globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
        builtins: Rc<HashMap<String, PyObjectRef>>,
        module_globals: Option<Rc<RefCell<HashMap<String, PyObjectRef>>>>,
    ) -> Self {
        let instr_count = code.instructions.len();
        Frame {
            fast_locals: vec![None; code.nlocals],
            code,
            locals: InternedMap::new(),
            globals,
            builtins,
            stack: SmallVec::new(),
            ip: 0,
            base_sp: 0,
            exception_handlers: Vec::new(),
            return_value: None,
            closure: Vec::new(),
            active_exception: None,
            attr_cache: vec![None; instr_count],
            global_cache: vec![None; instr_count],
            registers: Vec::new(),
            module_globals,
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
            let line_no = if instr_ip < self.code.instructions.len() { self.code.instructions[instr_ip].line_no.unwrap_or(0) } else { 0 };
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
    pub builtins: Rc<HashMap<String, PyObjectRef>>,
    pub modules: HashMap<String, PyObjectRef>,
    pub globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
    #[cfg(feature = "jit")]
    pub jit: RefCell<JitCompiler>,
    /// Execution profile counters — how many times each instruction ran.
    /// Indexed by (function_id, instruction_offset). Used by JIT to
    /// identify hot paths for native compilation.
    pub profile: RefCell<HashMap<usize, Vec<u32>>>,
    pub frame_pool: Vec<Frame>,
    /// Line number of the last instruction executed. Used for error reporting.
    pub last_error_line: Option<usize>,
    /// Type registry: maps type names to PyObject::Type objects.
    /// Used by builtin_type_of() to return real type objects instead of strings.
    pub type_registry: HashMap<String, PyObjectRef>,
    /// Current exception info for sys.exc_info()
    pub exc_type: Option<PyObjectRef>,
    pub exc_value: Option<PyObjectRef>,
    pub exc_traceback: Option<PyObjectRef>,
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

impl VirtualMachine {
    pub fn new() -> Self {
        Self::new_with_args(std::env::args().collect())
    }

    pub fn new_with_args(argv: Vec<String>) -> Self {
        let mut builtins = create_builtins();
        let globals_map = HashMap::from([
            ("__name__".to_string(), py_str("__main__")),
            ("__builtins__".to_string(), create_module("builtins", builtins.clone())),
        ]);
        let globals = Rc::new(RefCell::new(globals_map));

         let mut modules = HashMap::new();
         modules.insert("builtins".to_string(), create_module("builtins", builtins.clone()));
         modules.insert("math".to_string(), create_module("math", create_math_dict()));
         modules.insert("_codecs".to_string(), create_module("_codecs", create_codecs_dict()));

         let sys_dict = create_sys_dict(argv);
         modules.insert("sys".to_string(), create_module("sys", sys_dict.clone()));
          builtins.extend(sys_dict.clone());

         // Native os module
         let os_mod = create_module("os", create_os_dict());
         modules.insert("os".to_string(), os_mod.clone());
         // posix is the C extension behind os — alias it for importlib compatibility
         modules.insert("posix".to_string(), os_mod.clone());

         // Native os.path submodule (path manipulation functions)
         let os_path_mod = create_module("os.path", create_os_path_dict());
         // Wire path as a submodule attribute of the os parent module
         if let PyObject::Module { dict, .. } = &mut *os_mod.borrow_mut() {
             dict.insert("path".to_string(), os_path_mod.clone());
         }
         modules.insert("os.path".to_string(), os_path_mod);

         let pathlib_dict = create_pathlib_dict();
         modules.insert("pathlib".to_string(), create_module("pathlib", pathlib_dict));

         // Native urllib package (urllib.request, urllib.parse)
         let urllib_dict = create_urllib_dict();
         modules.insert("urllib".to_string(), create_module("urllib", urllib_dict));

         let json_dict = create_json_dict();
         modules.insert("json".to_string(), create_module("json", json_dict));

         let collections_dict = create_collections_dict();
          modules.insert("collections".to_string(), create_module("collections", collections_dict));

          let functools_dict = create_functools_dict();
          modules.insert("functools".to_string(), create_module("functools", functools_dict));

          let itertools_dict = create_itertools_dict();
          modules.insert("itertools".to_string(), create_module("itertools", itertools_dict));


          let datetime_dict = create_datetime_dict();
          modules.insert("datetime".to_string(), create_module("datetime", datetime_dict));

          let zoneinfo_dict = create_zoneinfo_dict();
          modules.insert("zoneinfo".to_string(), create_module("zoneinfo", zoneinfo_dict));

          let socket_dict = create_socket_dict();
          modules.insert("socket".to_string(), create_module("socket", socket_dict));

          let select_dict = create_select_dict();
          modules.insert("select".to_string(), create_module("select", select_dict));

          let re_dict = create_re_dict();
          modules.insert("re".to_string(), create_module("re", re_dict));

          let subprocess_dict = create_subprocess_dict();
          modules.insert("subprocess".to_string(), create_module("subprocess", subprocess_dict));

          // Native pickle module (basic stub)
          modules.insert("pickle".to_string(), create_module("pickle", create_pickle_dict()));

          // Native logging module
          modules.insert("logging".to_string(), create_module("logging", create_logging_dict()));
          modules.insert("logging.config".to_string(), create_module("logging.config", create_logging_config_dict()));

          // Native timeit module
          modules.insert("timeit".to_string(), create_module("timeit", create_timeit_dict()));

          let threading_dict = create_threading_dict();
          modules.insert("threading".to_string(), create_module("threading", threading_dict));

          // Native _thread module (CPython C extension replacement)
          modules.insert("_thread".to_string(), create_module("_thread", create_thread_module_dict()));

          // Native signal module (CPython C extension replacement)
          modules.insert("signal".to_string(), create_module("signal", create_signal_dict()));

          // Native gc module (CPython C extension replacement)
          modules.insert("gc".to_string(), create_module("gc", create_gc_dict()));

          // Native sysconfig module (CPython stdlib replacement)
          modules.insert("sysconfig".to_string(), create_module("sysconfig", create_sysconfig_dict()));

          // Native linecache module (CPython stdlib replacement)
          modules.insert("linecache".to_string(), create_module("linecache", create_linecache_dict()));

          // Native calendar module
          modules.insert("calendar".to_string(), create_module("calendar", create_calendar_dict()));

          // Native locale module
          modules.insert("locale".to_string(), create_module("locale", create_locale_dict()));

          // Native ssl module (CPython C extension replacement for urllib3 compatibility)
          modules.insert("ssl".to_string(), create_module("ssl", create_ssl_dict()));

          // Native time module
          modules.insert("time".to_string(), create_module("time", create_time_dict()));

          // Native C extension replacements for CPython stdlib compatibility
          let weakref_dict = create_weakref_dict();
          modules.insert("_weakref".to_string(), create_module("_weakref", weakref_dict.clone()));

          let collections_abc_dict = create_collections_abc_dict();
          modules.insert("_collections_abc".to_string(), create_module("_collections_abc", collections_abc_dict.clone()));
          // Pre-register collections.abc so the import chain walker finds it without needing __path__
          modules.insert("collections.abc".to_string(), create_module("collections.abc", collections_abc_dict));

          // Native weakref module (replaces CPython weakref.py)
          let mut weakref_mod_dict = weakref_dict; // Start from _weakref
          // Add WeakValueDictionary and WeakKeyDictionary as dict-like stubs
          weakref_mod_dict.insert("WeakValueDictionary".to_string(), create_weakref_weak_val_dict());
          weakref_mod_dict.insert("WeakKeyDictionary".to_string(), create_weakref_weak_key_dict());
          weakref_mod_dict.insert("WeakSet".to_string(), create_weakref_weak_set());
          modules.insert("weakref".to_string(), create_module("weakref", weakref_mod_dict));

          // Native copy module (replaces CPython copy.py which uses unsupported syntax)
          modules.insert("copy".to_string(), create_module("copy", create_copy_dict()));

          // Native types module (replaces CPython types.py)
          modules.insert("types".to_string(), create_module("types", create_types_dict()));

          // Native struct module for binary packing
          modules.insert("struct".to_string(), create_module("struct", create_struct_dict()));

          // Native bisect module for binary search
          modules.insert("bisect".to_string(), create_module("bisect", create_bisect_dict()));

          // Native heapq module for heap queue operations
          modules.insert("heapq".to_string(), create_module("heapq", create_heapq_dict()));

          // Native enum module
          modules.insert("enum".to_string(), create_module("enum", create_enum_dict()));

          // Native glob module
          modules.insert("glob".to_string(), create_module("glob", create_glob_dict()));

          // Native fnmatch module
          modules.insert("fnmatch".to_string(), create_module("fnmatch", create_fnmatch_dict()));

          // Native textwrap module
          modules.insert("textwrap".to_string(), create_module("textwrap", create_textwrap_dict()));

          // Native pprint module
          modules.insert("pprint".to_string(), create_module("pprint", create_pprint_dict()));

          // Native hashlib module
          modules.insert("hashlib".to_string(), create_module("hashlib", create_hashlib_dict()));

          // Native secrets module
          modules.insert("secrets".to_string(), create_module("secrets", create_secrets_dict()));

          // Native hmac module
          modules.insert("hmac".to_string(), create_module("hmac", create_hmac_dict()));

          // Native base64 module
          modules.insert("base64".to_string(), create_module("base64", create_base64_dict()));

          // Native binascii module
          modules.insert("binascii".to_string(), create_module("binascii", create_binascii_dict()));

          // Native uuid module
          modules.insert("uuid".to_string(), create_module("uuid", create_uuid_dict()));

          // Native string module (with capwords and Formatter)
          let mut string_dict = create_string_dict();
          let string_v2 = create_string_dict_v2();
          for (k, v) in string_v2 { string_dict.insert(k, v); }
          modules.insert("string".to_string(), create_module("string", string_dict));

          // Native colorsys module
          modules.insert("colorsys".to_string(), create_module("colorsys", create_colorsys_dict()));

          // Native wave module
          modules.insert("wave".to_string(), create_module("wave", create_wave_dict()));

          // Native numbers module (Number ABC stubs)
          modules.insert("numbers".to_string(), create_module("numbers", create_numbers_dict()));

          // Native ast module (literal_eval and node stubs)
          modules.insert("ast".to_string(), create_module("ast", create_ast_dict()));

          // Native sunau module (Sun AU audio format stubs)
          modules.insert("sunau".to_string(), create_module("sunau", create_sunau_dict()));

          // Native difflib module (with unified_diff)
          modules.insert("difflib".to_string(), create_module("difflib", create_difflib_dict()));

          // Native csv module
          modules.insert("csv".to_string(), create_module("csv", create_csv_dict()));

          // Native io module — DISABLED: CPython io.py is used instead (imports from _io)
          // modules.insert("io".to_string(), create_module("io", create_io_dict()));

          // Native statistics module
          modules.insert("statistics".to_string(), create_module("statistics", create_statistics_dict()));

          // Native contextlib module
          modules.insert("contextlib".to_string(), create_module("contextlib", create_contextlib_dict()));

          // Native decimal module
          modules.insert("decimal".to_string(), create_module("decimal", create_decimal_dict()));

          // Native fractions module
          modules.insert("fractions".to_string(), create_module("fractions", create_fractions_dict()));

          // Native platform module
          modules.insert("platform".to_string(), create_module("platform", create_platform_dict()));

          // Native getopt module
          modules.insert("getopt".to_string(), create_module("getopt", create_getopt_dict()));

          // Native getpass module
          modules.insert("getpass".to_string(), create_module("getpass", create_getpass_dict()));

          // Native errno module
          modules.insert("errno".to_string(), create_module("errno", create_errno_dict()));

          // Native _random module (C extension stub for CPython's random.py)
          modules.insert("_random".to_string(), create_module("_random", create_random_cmodule_dict()));

          // Native shutil module
          modules.insert("shutil".to_string(), create_module("shutil", create_shutil_dict()));

          // Native graphlib module
          modules.insert("graphlib".to_string(), create_module("graphlib", create_graphlib_dict()));

          // Native pdb module
          modules.insert("pdb".to_string(), create_module("pdb", create_pdb_dict()));

          // Native traceback module
          modules.insert("traceback".to_string(), create_module("traceback", create_traceback_dict()));

          // Native warnings module
          modules.insert("warnings".to_string(), create_module("warnings", create_warnings_dict()));

          // Native abc module
          modules.insert("abc".to_string(), create_module("abc", create_abc_dict()));

          // Native typing module (type annotation stubs)
          // Comment out native typing - use Lib/typing.py instead
          // modules.insert("typing".to_string(), create_module("typing", create_typing_dict()));

          // Native pickle module
          modules.insert("pickle".to_string(), create_module("pickle", create_pickle_dict()));

          // Native logging module
          modules.insert("logging".to_string(), create_module("logging", create_logging_dict()));

          // Native timeit module
          modules.insert("timeit".to_string(), create_module("timeit", create_timeit_dict()));

          // Native json.tool module
          modules.insert("json.tool".to_string(), create_module("json.tool", create_json_tool_dict()));

          // Native cmath module (complex math: sqrt, sin, cos)
          modules.insert("cmath".to_string(), create_module("cmath", create_cmath_dict()));

          // Native gzip module
          modules.insert("gzip".to_string(), create_module("gzip", create_gzip_dict()));

          // Native zlib module
          modules.insert("zlib".to_string(), create_module("zlib", create_zlib_dict()));

          // Native tarfile module
          modules.insert("tarfile".to_string(), create_module("tarfile", create_tarfile_dict()));

          // Native zipfile module (read-only)
          modules.insert("zipfile".to_string(), create_module("zipfile", create_zipfile_dict()));

          // Native hashlib_extra module
          modules.insert("hashlib_extra".to_string(), create_module("hashlib_extra", create_hashlib_extra_dict()));

          // Native dataclasses module
          modules.insert("dataclasses".to_string(), create_module("dataclasses", create_dataclasses_dict()));

          // Native operator module
          modules.insert("operator".to_string(), create_module("operator", create_operator_dict()));

          // Native reprlib module
          modules.insert("reprlib".to_string(), create_module("reprlib", create_reprlib_dict()));

          // Native array module
          modules.insert("array".to_string(), create_module("array", create_array_dict()));

          // Native shelve module (persistent dict wrapper)
          modules.insert("shelve".to_string(), create_module("shelve", create_shelve_dict()));

          // Native mimetypes module
          modules.insert("mimetypes".to_string(), create_module("mimetypes", create_mimetypes_dict()));

          // Native dis module for bytecode disassembly
          modules.insert("dis".to_string(), create_module("dis", create_dis_dict()));

          // Native http module (HTTPStatus enum)
          let http_mod = create_module("http", create_http_dict());
          modules.insert("http".to_string(), http_mod.clone());

          // Native http.client submodule (HTTPConnection, HTTPResponse)
          let http_client_mod = create_module("http.client", create_http_client_dict());
          // Wire client as a submodule attribute of the http parent module
          if let PyObject::Module { dict, .. } = &mut *http_mod.borrow_mut() {
              dict.insert("client".to_string(), http_client_mod.clone());
          }
          modules.insert("http.client".to_string(), http_client_mod);

          // Native smtplib module (SMTP stub)
          modules.insert("smtplib".to_string(), create_module("smtplib", create_smtplib_dict()));

          // Native html module (escape/unescape)
          let html_mod = create_module("html", create_html_dict());
          modules.insert("html".to_string(), html_mod.clone());

          // Native html.entities module (html5 entity map)
          let html_entities_mod = create_module("html.entities", create_html_entities_dict());
          // Wire entities as a submodule attribute of the html parent module
          if let PyObject::Module { dict, .. } = &mut *html_mod.borrow_mut() {
              dict.insert("entities".to_string(), html_entities_mod.clone());
          }
          modules.insert("html.entities".to_string(), html_entities_mod);

          // Native html.parser module (HTMLParser stub)
          let html_parser_mod = create_module("html.parser", create_html_parser_dict());
          // Wire parser as a submodule attribute of the html parent module
          if let PyObject::Module { dict, .. } = &mut *html_mod.borrow_mut() {
              dict.insert("parser".to_string(), html_parser_mod.clone());
          }
          modules.insert("html.parser".to_string(), html_parser_mod);

          // Native unittest module (stub with TestCase)
          modules.insert("unittest".to_string(), create_module("unittest", create_unittest_dict()));

          // Native doctest module (stub with TestResults and testmod)
          modules.insert("doctest".to_string(), create_module("doctest", create_doctest_dict()));

          // Native email module (stub with EmailMessage)
          let email_mod = create_module("email", create_email_dict());
          modules.insert("email".to_string(), email_mod.clone());

          // Native email.mime.text submodule (MIMEText stub)
          let email_mime_text_mod = create_module("email.mime.text", create_email_mime_text_dict());
          // Create email.mime intermediate submodule and wire it under email
          let email_mime_mod = create_module("email.mime", HashMap::new());
          {
              let mut email_mut = email_mod.borrow_mut();
              if let PyObject::Module { dict: email_dict, .. } = &mut *email_mut {
                  email_dict.insert("mime".to_string(), email_mime_mod.clone());
              }
          }
          {
              let mut mime_mut = email_mime_mod.borrow_mut();
              if let PyObject::Module { dict: mime_dict, .. } = &mut *mime_mut {
                  mime_dict.insert("text".to_string(), email_mime_text_mod.clone());
              }
          }
          modules.insert("email.mime".to_string(), email_mime_mod);
          modules.insert("email.mime.text".to_string(), email_mime_text_mod);

          // Native email.utils submodule (formatdate, etc.)
          let email_utils_mod = create_module("email.utils", create_email_utils_dict());
          {
              let mut email_mut = email_mod.borrow_mut();
              if let PyObject::Module { dict: email_dict, .. } = &mut *email_mut {
                  email_dict.insert("utils".to_string(), email_utils_mod.clone());
              }
          }
          modules.insert("email.utils".to_string(), email_utils_mod);

          // Native email.header submodule (Header class)
          let email_header_mod = create_module("email.header", create_email_header_dict());
          {
              let mut email_mut = email_mod.borrow_mut();
              if let PyObject::Module { dict: email_dict, .. } = &mut *email_mut {
                  email_dict.insert("header".to_string(), email_header_mod.clone());
              }
          }
          modules.insert("email.header".to_string(), email_header_mod);

          // Native configparser module
          modules.insert("configparser".to_string(), create_module("configparser", create_configparser_dict()));

          // Native xml.etree.ElementTree module
          let xml_etree_mod = create_module("xml.etree.ElementTree", create_xml_etree_dict());
          modules.insert("xml.etree.ElementTree".to_string(), xml_etree_mod.clone());
          // Native xml module (empty package)
          let xml_mod = create_module("xml", create_xml_dict());
          // Wire etree as a submodule of xml
          if let PyObject::Module { dict: xml_el_dict, .. } = &mut *xml_mod.borrow_mut() {
              xml_el_dict.insert("etree".to_string(), xml_etree_mod.clone());
          }
          modules.insert("xml".to_string(), xml_mod);

          // Native this module (Zen of Python)
          modules.insert("this".to_string(), create_module("this", create_this_dict()));

          // Native argparse module (ArgumentParser stub)
          modules.insert("argparse".to_string(), create_module("argparse", create_argparse_dict()));

          // Native _imp module (CPython C extension replacement needed by importlib._bootstrap)
          modules.insert("_imp".to_string(), create_module("_imp", create_imp_dict()));
          // Native _warnings module (CPython C extension replacement)
          modules.insert("_warnings".to_string(), create_module("_warnings", create_warnings_c_dict()));
          // Native marshal module (CPython C extension replacement)
          modules.insert("marshal".to_string(), create_module("marshal", create_marshal_dict()));
          // Native zipimport module stub
          modules.insert("zipimport".to_string(), create_module("zipimport", create_zipimport_dict()));
          // Native _io module (CPython C extension replacement needed by importlib._bootstrap_external)
          modules.insert("_io".to_string(), create_module("_io", create_io_module_dict()));
          // Native queue module (Queue backed by PyObject::Queue)
          modules.insert("queue".to_string(), create_module("queue", create_queue_dict()));

          // Native importlib stub module
          let importlib_mod = create_module("importlib", create_importlib_dict());
          // Wire importlib.resources as a submodule
          {
              let resources_mod = create_module("importlib.resources", create_importlib_resources_dict());
              if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                  dict.insert("resources".to_string(), resources_mod.clone());
              }
              modules.insert("importlib.resources".to_string(), resources_mod);
          }
          // Wire importlib.util as a submodule
          {
              let util_mod = create_module("importlib.util", create_importlib_util_dict());
              if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                  dict.insert("util".to_string(), util_mod.clone());
              }
              modules.insert("importlib.util".to_string(), util_mod);
          }
          // Add __path__ so dotted imports like importlib.machinery can find filesystem submodules
          {
              if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                  dict.insert("__path__".to_string(), py_list(vec![py_str(&format!("{}/importlib", find_lib_dir()))]));
              }
          }
          modules.insert("importlib".to_string(), importlib_mod);

          modules.insert("inspect".to_string(), create_module("inspect", create_inspect_dict()));

          // Native __future__ module (needed by requests, etc.)
          modules.insert("__future__".to_string(), create_module("__future__", create_future_dict()));

          // Native asyncio module (basic event loop)
          modules.insert("asyncio".to_string(), create_module("asyncio", create_asyncio_dict()));

          // Native atexit module (register/unregister exit callbacks)
          modules.insert("atexit".to_string(), create_module("atexit", create_atexit_dict()));

          // Native contextvars module (ContextVar with thread-local storage)
          modules.insert("contextvars".to_string(), create_module("contextvars", create_contextvars_dict()));

          // Native unicodedata module (basic Unicode category/normalize)
          modules.insert("unicodedata".to_string(), create_module("unicodedata", create_unicodedata_dict()));

          // Native profile module
          modules.insert("profile".to_string(), create_module("profile", create_profile_dict()));

          // Native cProfile module
          modules.insert("cProfile".to_string(), create_module("cProfile", create_cprofile_dict()));

          // Native resource module (POSIX resource usage stubs)
          modules.insert("resource".to_string(), create_module("resource", create_resource_dict()));

          // Native trace module (code tracing / coverage stubs)
          modules.insert("trace".to_string(), create_module("trace", create_trace_dict()));

          // Native _concurrent module (concurrent.futures backend)
          let concurrent_futures_mod = create_module("concurrent.futures", create_concurrent_futures_dict());
          // Create intermediate concurrent package and wire futures under it
          let concurrent_mod = create_module("concurrent", HashMap::new());
          {
              let mut conc_mut = concurrent_mod.borrow_mut();
              if let PyObject::Module { dict, .. } = &mut *conc_mut {
                  dict.insert("futures".to_string(), concurrent_futures_mod.clone());
              }
          }
          modules.insert("concurrent".to_string(), concurrent_mod);
          modules.insert("concurrent.futures".to_string(), concurrent_futures_mod);

          // Native sqlite3 module (requires --features sqlite3)
          #[cfg(feature = "sqlite3")]
          modules.insert("sqlite3".to_string(), create_module("sqlite3", create_sqlite3_dict()));

          // Populate sys.path with default search paths
        if let PyObject::List(path_list) = &mut *sys_dict.get("path").unwrap().borrow_mut() {
            path_list.push(py_str("."));
            path_list.push(py_str(&find_lib_dir()));

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
         // Populate sys.modules with already-loaded modules
         if let PyObject::Dict(mod_dict) = &mut *sys_dict.get("modules").unwrap().borrow_mut() {
             for (name, module) in &modules {
                 mod_dict.set(py_str(name), module.clone()).ok();
             }
         }

         let mut vm = VirtualMachine {
              frames: Vec::new(),
              builtins: Rc::new(builtins),
              modules,
              globals,
              #[cfg(feature = "jit")]
             jit: RefCell::new(JitCompiler::new()),
              profile: RefCell::new(HashMap::new()),
               last_error_line: None,
               frame_pool: Vec::new(),
               type_registry: HashMap::new(),
               exc_type: None,
               exc_value: None,
               exc_traceback: None,
           };
         vm.populate_type_registry();
         vm.install_source_defined_stdlib("collections", crate::modules::COLLECTIONS_USER_TYPES_SOURCE, &["UserList", "UserDict", "UserString", "Counter", "defaultdict"]);
         vm.install_source_defined_stdlib("contextlib", crate::modules::CONTEXTLIB_SOURCE, &["ContextDecorator"]);
         vm.install_source_defined_stdlib("functools", crate::modules::FUNCTOOLS_EXTRA_SOURCE, &["lru_cache", "cache"]);
         vm
    }

    /// Some stdlib classes are far easier (and more correct) to express as
    /// real Python source — the same way CPython's own stdlib does it — than
    /// as hand-written Rust closures (e.g. anything relying on composition
    /// over a `self.data` attribute, decorators, or `with`). This compiles
    /// and runs that source against this already-constructed VM (never a
    /// nested one — building a VM calls this same constructor, which would
    /// recurse infinitely), extracts the requested names, merges them into
    /// the given already-registered native module's dict, and then removes
    /// them from `self.globals` again — `run()` executes against the VM's
    /// real globals (shared with whatever user script runs next), so
    /// without this cleanup step every such name would leak into every
    /// script's top-level namespace with no import required.
    fn install_source_defined_stdlib(&mut self, module_name: &str, source: &str, names: &[&str]) {
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
        if self.run(code).is_err() {
            return;
        }
        let extracted: Vec<(String, PyObjectRef)> = {
            let mut globals = self.globals.borrow_mut();
            names.iter().filter_map(|name| globals.remove(*name).map(|v| (name.to_string(), v))).collect()
        };
        if let Some(module) = self.modules.get(module_name) {
            if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                for (name, obj) in extracted {
                    dict.insert(name, obj);
                }
            }
        }
    }

    fn acquire_frame(
        &mut self,
        code: Rc<CodeObject>,
        globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
        builtins: Rc<HashMap<String, PyObjectRef>>,
        module_globals: Option<Rc<RefCell<HashMap<String, PyObjectRef>>>>,
    ) -> Frame {
        if let Some(mut frame) = self.frame_pool.pop() {
            let nlocals = code.nlocals;
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
            frame.return_value = None;
            frame.closure.clear();
            frame.active_exception = None;
            frame.attr_cache.clear();
            frame.global_cache.clear();
            frame.registers.clear();
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
        // JIT compilation disabled — using stable interpreter path only
        let frame = self.acquire_frame(
            Rc::new(code),
            self.globals.clone(),
            Rc::clone(&self.builtins),
            None,
        );
        self.frames.push(frame);
        let result = self.execute();
        if let Some(frame) = self.frames.pop() {
            self.release_frame(frame);
        }
        result
    }

    pub fn exec_code(&mut self, code: CodeObject, globals: Option<Rc<RefCell<HashMap<String, PyObjectRef>>>>) -> PyResult<PyObjectRef> {
        let g = globals.unwrap_or_else(|| self.globals.clone());
        let frame = self.acquire_frame(Rc::new(code), g, Rc::clone(&self.builtins), None);
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
                dict: HashMap::new(),
                bases: vec![],
                mro: vec![],
            });
            self.type_registry.insert(name.to_string(), type_obj);
        }
    }

    pub fn import_module_from_file(&mut self, name: &str) -> Result<PyObjectRef, String> {
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
                                let p = dict.get("__path__").and_then(|pl| {
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
                        for candidate in &[
                            format!("{}/{}.py", base_trimmed, child),
                            format!("{}/{}/__init__.py", base_trimmed, child),
                        ] {
                            if let Ok(source) = std::fs::read_to_string(candidate) {
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
                                        PyObject::Module { dict, .. } => dict.get("modules").cloned(),
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
                                            dict.insert(child_name, module.clone());
                                        }
                                    }
                                }
                                current_name = full_name;
                                parent_path = None;
                                break;
                            }
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
            if let Ok(source) = std::fs::read_to_string(&py_path) {
                let empty_mod = create_module(name, HashMap::new());
                self.modules.insert(name.to_string(), empty_mod.clone());
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get("modules").cloned() {
                            mod_dict.borrow_mut().set_attribute(name, empty_mod.clone()).ok();
                        }
                    }
                }
                let module = self.exec_module_source(&source, &py_path, name)?;
                self.modules.insert(name.to_string(), module.clone());
                // Wire submodule into parent module namespace and update sys.modules
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get("modules").cloned() {
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
                            dict.insert(child_name, module.clone());
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
            if let Ok(source) = std::fs::read_to_string(&init_path) {
                let pkg_dir = std::path::Path::new(&init_path).parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let mut empty_dict = HashMap::new();
                empty_dict.insert("__path__".to_string(), py_list(vec![py_str(&pkg_dir)]));
                empty_dict.insert("__package__".to_string(), py_str(name));
                let empty_mod = create_module(name, empty_dict);
                self.modules.insert(name.to_string(), empty_mod.clone());
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get("modules").cloned() {
                            mod_dict.borrow_mut().set_attribute(name, empty_mod.clone()).ok();
                        }
                    }
                }
                let module = self.exec_module_source(&source, &init_path, name)?;
                self.modules.insert(name.to_string(), module.clone());
                // Update sys.modules with the loaded module (overwrites empty stub)
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get("modules").cloned() {
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
        Err(format!("No module named '{}'", name))
    }

    fn get_sys_path(&self) -> Vec<String> {
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(path_list) = dict.get("path") {
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

    fn exec_module_source(&mut self, source: &str, path: &str, name: &str) -> Result<PyObjectRef, String> {
        // ── .pyc cache support ─────────────────────────────────────────
        // Try to load a previously-compiled .pyc file. If valid (matching
        // magic + version + source timestamp), skip parsing and compilation.
        const PYC_MAGIC: u32 = 0x52535079; // "RSPy"
        const PYC_VERSION: u16 = 1;

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
                    .map_err(|e| format!("Parse error in '{}': {}", name, e))?;
                drop(parser);  // Free parser memory (AST is now in `program`)

                let mut compiler = crate::compiler::Compiler::new();
                let compiled = compiler.compile(&program, path)
                    .map_err(|e| format!("Compile error: {}", e))?;
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
        let mut globals_map = HashMap::from([
            ("__name__".to_string(), py_str(name)),
            ("__file__".to_string(), py_str(path)),
            ("__builtins__".to_string(), create_module("builtins", self.builtins.as_ref().clone())),
        ]);
        if is_package {
            if let Some(pkg_dir) = std::path::Path::new(path).parent() {
                let pkg_dir_str = pkg_dir.to_string_lossy().to_string();
                globals_map.insert("__path__".to_string(), py_list(vec![py_str(&pkg_dir_str)]));
                globals_map.insert("__package__".to_string(), py_str(name));
            }
        } else {
            // For non-package modules, __package__ should be set to the parent package name
            // (e.g., "django.apps" for "django.apps.registry") so relative imports work
            let pkg = name.rfind('.').map(|dot| &name[..dot]).unwrap_or("");
            globals_map.insert("__package__".to_string(), 
                if pkg.is_empty() { py_str("") } else { py_str(pkg) });
        }
        let module_globals = Rc::new(RefCell::new(globals_map));
        // Register module in sys.modules BEFORE executing (needed for sys.modules[__name__] checks)
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(sm) = dict.get("modules").cloned() {
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
        self.exec_code(code, Some(Rc::clone(&module_globals))).map_err(|e| {
            format!("{}", e)
        })?;
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
                dict.extend(globals_copy);
            }
            return Ok(existing);
        }
        Ok(create_module(name, globals_copy))
    }

    /// Try to execute a simple function without creating a Frame.
    /// Returns Some(result) if the function was simple enough, None otherwise.
    fn try_exec_simple(code: &CodeObject, args: &[PyObjectRef]) -> Option<PyResult<PyObjectRef>> {
        if code.vararg_name.is_some() || code.kwarg_name.is_some() || code.num_defaults > 0 {
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
                    let const_val = code.consts.get(instr.arg as usize)?;
                    let obj = match const_val {
                        ConstValue::None => py_none(),
                        ConstValue::Bool(b) => py_bool(*b),
                        ConstValue::Int(s) => {
                            let s_trim = s.trim_start_matches('_');
                            if let Some(oct) = s_trim.strip_prefix("0o").or_else(|| s_trim.strip_prefix("0O")) {
                                if let Ok(n) = i64::from_str_radix(oct, 8) { py_int(n) }
                                else { let n = num_bigint::BigInt::parse_bytes(oct.as_bytes(), 8)?; PyObjectRef::imm(PyObject::Int(n)) }
                            } else if let Some(hex) = s_trim.strip_prefix("0x").or_else(|| s_trim.strip_prefix("0X")) {
                                if let Ok(n) = i64::from_str_radix(hex, 16) { py_int(n) }
                                else { let n = num_bigint::BigInt::parse_bytes(hex.as_bytes(), 16)?; PyObjectRef::imm(PyObject::Int(n)) }
                            } else if let Some(bin) = s_trim.strip_prefix("0b").or_else(|| s_trim.strip_prefix("0B")) {
                                if let Ok(n) = i64::from_str_radix(bin, 2) { py_int(n) }
                                else { let n = num_bigint::BigInt::parse_bytes(bin.as_bytes(), 2)?; PyObjectRef::imm(PyObject::Int(n)) }
                            } else if let Ok(n) = s.parse::<i64>() { py_int(n) }
                            else { let n: num_bigint::BigInt = s.parse().ok()?; PyObjectRef::imm(PyObject::Int(n)) }
                        }
                        ConstValue::Float(s) => py_float(s.parse().ok()?),
                        ConstValue::String(s) => py_str(s),
                        _ => return None,
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
                        10 => py_bit_and(&left, &right),
                        11 => py_bit_xor(&left, &right),
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
        let result = self.execute_inner();
        // Store exception info for sys.exc_info()
        if let Err(ref e) = result {
            self.exc_type = Some(py_str(&e.type_name()));
            self.exc_value = Some(py_str(&format!("{}", e)));
        }
        result
    }

    fn execute_inner(&mut self) -> PyResult<PyObjectRef> {
        loop {
            let result = self.execute_instruction();
            match result {
                Ok(None) => continue,
                Ok(Some(val)) => return Ok(val),
                Err(e) => {
                    if matches!(&e, PyError::SystemExit(_)) {
                        return Err(e);
                    }
                    if !self.handle_exception(&e) {
                        return Err(e);
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
        self.last_error_line = self.frames[fi].code.instructions[ip].line_no;
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
                let const_val = self.frames[fi].code.consts.get(const_idx).ok_or_else(|| {
                    PyError::runtime_error(format!("constant index out of range: {}", const_idx))
                })?.clone();
                let obj = match const_val {
                    ConstValue::None => py_none(),
                    ConstValue::Bool(b) => py_bool(b),
                    ConstValue::Int(s) => {
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
                        } else if let Ok(n) = s.parse::<i64>() {
                            py_int(n)  // uses small int cache
                        } else {
                            let n: BigInt = s.parse().map_err(|_| {
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
                        // Create a string representation matching Python's complex repr
                        if imag.starts_with('-') {
                            py_str(&format!("({}{}j)", real, imag))
                        } else {
                            py_str(&format!("({}+{}j)", real, imag))
                        }
                    }
                    ConstValue::Code(code) => {
                        PyObjectRef::imm(PyObject::Code(code))
                    }
                    ConstValue::Tuple(items) => {
                        let objs: Vec<PyObjectRef> = items.into_iter().map(|s| py_str(&s)).collect();
                        PyObjectRef::imm(PyObject::Tuple(objs))
                    }
                };
                self.frames[fi].push(obj);
            }

            Opcode::LOAD_NAME => {
                let name_idx = arg as usize;
                let name = &self.frames[fi].code.names[name_idx];
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.get_local(name).cloned()
                        .or_else(|| f.globals.borrow().get(name).cloned())
                        .or_else(|| {
                            // Check module_globals (enclosing module scope for class bodies)
                            f.module_globals.as_ref()
                                .and_then(|mg| mg.borrow().get(name).cloned())
                        })
                        .or_else(|| f.builtins.get(name).cloned())
                };
                match val {
                    Some(v) => self.frames[fi].push(v),
                    None => return Err(PyError::name_error(format!("name '{}' is not defined", name))),
                }
            }

            Opcode::STORE_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames[fi].pop()?;
                self.frames[fi].globals.borrow_mut().insert(name, val);
            }

            Opcode::LOAD_FAST => {
                let var_idx = arg as usize;
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.fast_locals.get(var_idx).and_then(|v| v.clone())
                };
                match val {
                    Some(v) => self.frames[fi].push(v),
                    None => return Err(PyError::name_error(format!("local variable '{}' referenced before assignment", 
                        self.frames[fi].code.varnames.get(var_idx).map_or("?", |s| &**s)))),
                }
            }

            Opcode::STORE_FAST => {
                let var_idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let frame = &mut self.frames[fi];
                if var_idx < frame.fast_locals.len() {
                    frame.fast_locals[var_idx] = Some(val.clone());
                }
                let name = frame.code.varnames.get(var_idx).ok_or_else(|| {
                    PyError::runtime_error("varname index out of range")
                })?.clone();
                frame.insert_local(&name, val);
            }

            Opcode::LOAD_GLOBAL => {
                let instr_ip = self.frames[fi].ip - 1;  // already incremented
                // Check inline cache first
                if let Some(cached) = self.frames[fi].global_cache.get(instr_ip).and_then(|c| c.clone()) {
                    self.frames[fi].push(cached);
                } else {
                    let name_idx = arg as usize;
                    let name = &self.frames[fi].code.names[name_idx];
                    let val = {
                        let f = &self.frames[self.frames.len() - 1];
                        let v = f.globals.borrow().get(name).cloned()
                            .or_else(|| f.module_globals.as_ref()
                                .and_then(|mg| mg.borrow().get(name).cloned()))
                            .or_else(|| f.builtins.get(name).cloned());
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
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames[fi].pop()?;
                self.frames[fi].globals.borrow_mut().insert(name, val);
            }

            Opcode::LOAD_DEREF => {
                let idx = arg as usize;
                let (cell_ref, is_freevar, name_str): (Option<PyObjectRef>, bool, String) = {
                    let f = &self.frames[fi];
                    let code = &f.code;
                    if idx < code.cellvars.len() {
                        let name = &code.cellvars[idx];
                        let var_idx = code.varnames.iter().position(|n| n == name)
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
                        globals.get(&name_str).cloned()
                    };
                    if let Some(v) = val {
                        self.frames[fi].push(v);
                    } else {
                        let val = self.frames[fi].builtins.get(&name_str).cloned();
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
                    let var_idx = self.frames[fi].code.varnames.iter().position(|n| n == name)
                        .ok_or_else(|| PyError::runtime_error("variable not found"))?;
                    if var_idx < self.frames[fi].fast_locals.len() {
                        if let Some(cell) = self.frames[fi].fast_locals[var_idx].clone() {
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
                    if let Some(cell) = self.frames[fi].closure.get(fv_idx).cloned() {
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
                let name = self.frames[fi].code.varnames[var_idx].clone();
                self.frames[fi].remove_local(&name);
            }

            Opcode::DELETE_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names[name_idx].clone();
                self.frames[fi].globals.borrow_mut().remove(&name);
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
                    self.frames[fi].registers = vec![None; 256];
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
                        if imag.starts_with('-') {
                            py_str(&format!("({}{}j)", real, imag))
                        } else {
                            py_str(&format!("({}+{}j)", real, imag))
                        }
                    }
                    ConstValue::Code(code) => PyObjectRef::imm(PyObject::Code(code)),
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
                    .ok_or_else(|| PyError::name_error("local variable referenced before assignment"))?;
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
                let name = self.frames[fi].code.varnames.get(var_idx).ok_or_else(|| {
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
                    10 => py_bit_and(&a, &b),
                    11 => py_bit_xor(&a, &b),
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
                let name = &self.frames[fi].code.names[name_idx];
                // Check inline cache first
                let instr_ip = self.frames[fi].ip - 1;
                if let Some(cached) = self.frames[fi].global_cache.get(instr_ip).and_then(|c| c.clone()) {
                    if dst < self.frames[fi].registers.len() {
                        self.frames[fi].registers[dst] = Some(cached);
                    }
                } else {
                    let val = self.frames[fi].globals.borrow().get(name).cloned()
                        .or_else(|| self.frames[fi].builtins.get(name).cloned());
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
                let mut defaults = Vec::new();
                for _ in 0..n_defaults {
                    defaults.push(self.frames[fi].pop()?);
                }
                defaults.reverse();
                let code_obj = self.frames[fi].pop()?;
                let code = match &*code_obj.borrow() {
                    PyObject::Code(c) => c.as_ref().clone(),
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
                let name = if !code.name.is_empty() {
                    code.name.clone()
                } else {
                    "<function>".to_string()
                };
                // Use module_globals when available (class body execution) so that
                // functions defined inside a class body capture the module's globals
                // (e.g. 'empty' from django.utils.functional) rather than the class
                // namespace. Falls back to the frame's globals for module-level code
                // and regular function calls.
                let globals = self.frames[fi].module_globals.clone()
                    .unwrap_or_else(|| self.frames[fi].globals.clone());
                let code_obj = code.clone();
                let func = PyObjectRef::new(PyObject::Function {
                    code: code_obj.clone(),
                    globals,
                    name,
                    defaults,
                    closure,
                    dict: HashMap::new(),
                    jit_ptr: std::cell::Cell::new(0),
                    jit_consts: std::cell::RefCell::new(Vec::new()),
                });
                // Set __code__ and __module__ on the function
                if let PyObject::Function { dict, .. } = &mut *func.borrow_mut() {
                    dict.insert("__code__".to_string(), PyObjectRef::imm(PyObject::Code(Box::new(code_obj))));
                }
                if let Some(ref mg) = self.frames[fi].module_globals {
                    let mg = mg.borrow();
                    if let Some(module_name) = mg.get("__name__") {
                        if let PyObject::Str(s) = &*module_name.borrow() {
                            if let PyObject::Function { dict, .. } = &mut *func.borrow_mut() {
                                dict.insert("__module__".to_string(), py_str(s));
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
                let op = arg;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
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
                    PyObject::Tuple(v) => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: v.clone(), index: 0 }));
                    }
                    PyObject::Str(s) => {
                        let chars: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: chars, index: 0 }));
                    }
                    PyObject::Set(s) => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: s.to_vec(), index: 0 }));
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
                            Err(e) if matches!(&e, PyError::StopIteration) => {
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
                        PyObject::EnumerateIter { items, pos, .. } => *pos >= items.len(),
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
                                *current += *step;
                                v
                            }
                            PyObject::EnumerateIter { items, pos, start } => {
                                let idx = *start + *pos;
                                let val = items[*pos].clone();
                                *pos += 1;
                                py_tuple(vec![py_int(idx as i64), val])
                            }
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
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let obj = self.frames[fi].pop()?;
                let result = {
                    let obj_borrowed = obj.borrow();
                    match &*obj_borrowed {
                        PyObject::Super { cls: _, obj: _super_obj } => {
                            // super(cls, obj).attr: walk MRO of obj's type, starting after cls
                            drop(obj_borrowed);
                            let attr = obj.borrow().get_attribute(&name)?;
                            Ok(attr)
                        }
                        PyObject::Instance { dict, typ } => {
                            // Inline attribute cache: skip full lookup if cached with matching type tag
                            let type_tag = typ.get_id() as u64;
                            let cached = self.frames[fi].attr_cache.get(name_idx)
                                .and_then(|entry| entry.as_ref())
                                .filter(|(tag, _)| *tag == type_tag)
                                .map(|(_, val)| val.clone());
                            if let Some(cached_val) = cached {
                                self.frames[fi].push(cached_val);
                                return Ok(None);
                            }
                            if name == "__dict__" {
                                // Return a live Dict view backed by the instance's HashMap.
                                // NATIVE_BACKING_KEY is internal bookkeeping
                                // (see native_backing_of) and must not leak
                                // into user-visible introspection.
                                let mut pd = crate::object::PyDict {
                                    buckets: std::collections::HashMap::new(),
                                    size: 0,
                                    instance_ref: None,
                                };
                                for (k, v) in dict.iter() {
                                    if k == crate::object::NATIVE_BACKING_KEY { continue; }
                                    let key = py_str(k);
                                    let h = key.hash().unwrap_or(0);
                                    pd.buckets.entry(h).or_default().push((key, v.clone()));
                                    pd.size += 1;
                                }
                                drop(obj_borrowed);
                                pd.instance_ref = Some(obj.clone());
                                self.frames[fi].push(PyObjectRef::new(PyObject::Dict(pd)));
                                return Ok(None);
                            }
                            if name == "__class__" {
                                let cls = typ.clone();
                                drop(obj_borrowed);
                                self.frames[fi].push(cls);
                                return Ok(None);
                            }
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
                                            PyObject::Property { getter: Some(g), .. } => {
                                                drop(typ_ref);
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
                                            PyObject::Function { .. } => {
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
                                                        let descriptor_args = vec![val.clone(), obj.clone(), cls];
                                                        return Some(self.call_function(__get__, descriptor_args, vec![]).unwrap_or_else(|_| val.clone()));
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
                            match attr {
                                Some(val) => {
                                    // Cache attribute for future accesses
                                    if name_idx < self.frames[fi].attr_cache.len() {
                                        self.frames[fi].attr_cache[name_idx] = Some((type_tag, val.clone()));
                                    }
                                    Ok(val)
                                }
                                None => {
                                    // Check for __getattr__ method on type before erroring
                                    let typ_ref = typ.borrow();
                                    if let PyObject::Type { dict: type_dict, .. } = &*typ_ref {
                                        if let Some(getattr_method) = type_dict.get_str("__getattr__").cloned() {
                                            drop(typ_ref);
                                            drop(obj_borrowed);
                                            self.call_function(getattr_method, vec![obj.clone(), py_str(&name)], vec![])
                                        } else {
                                            Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", obj_borrowed.type_name(), name)))
                                        }
                                    } else {
                                        Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", obj_borrowed.type_name(), name)))
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
                                let attr = obj_borrowed.get_attribute(&name).or_else(|_| {
                                    Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", obj_borrowed.type_name(), name)))
                                })?;
                                drop(obj_borrowed);
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
                                let is_builtin_method = matches!(&*attr.borrow(), PyObject::BuiltinMethod { .. });
                                let is_function = matches!(&*attr.borrow(), PyObject::Function { .. });
                                if is_builtin_method {
                                    let (n, func) = {
                                        let b = attr.borrow();
                                        if let PyObject::BuiltinMethod { name: n, func, .. } = &*b {
                                            (n.clone(), *func)
                                        } else { unreachable!() }
                                    };
                                    // Cache for next time
                                    ATTR_CACHE.with(|c| { c.borrow_mut().insert((type_name.clone(), n.clone()), func); });
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
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;

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
                obj.borrow_mut().set_attribute(&name, val)?;
            }

            Opcode::STORE_SUBSCR => {
                let val = self.frames[fi].pop()?;
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                py_setitem(&obj, &index, val)?;
            }

            Opcode::DELETE_SUBSCR => {
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                py_delitem(&obj, &index)?;
            }

            Opcode::DELETE_ATTR => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
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
                let items = {
                    let val_ref = val.borrow();
                    match &*val_ref {
                        PyObject::List(v) => v.clone(),
                        PyObject::Tuple(v) => v.clone(),
                        _ => {
                            return Err(PyError::runtime_error(
                                "LIST_EXTEND requires a list or tuple",
                            ));
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
                let items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => {
                            if v.len() != count {
                                return Err(PyError::value_error(format!(
                                    "cannot unpack {} items into {} values", v.len(), count
                                )));
                            }
                            v.clone()
                        }
                        _ => return Err(PyError::type_error("cannot unpack non-iterable")),
                    }
                };
                for item in items.into_iter().rev() {
                    self.frames[fi].push(item);
                }
            }

            Opcode::UNPACK_EX => {
                let before = (arg >> 8) as usize;
                let after = (arg & 0xFF) as usize;
                let total = before + after + 1; // +1 for the starred item
                let seq = self.frames[fi].pop()?;
                let items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => {
                            if v.len() < total {
                                return Err(PyError::value_error(format!(
                                    "cannot unpack {} items into {} values", v.len(), total
                                )));
                            }
                            v.clone()
                        }
                        _ => return Err(PyError::type_error("cannot unpack non-iterable")),
                    }
                };
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
                    self.frames[fi].active_exception = Some(exc);
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
                let typ_name = match &*exc.borrow() {
                    PyObject::Exception { typ, .. } => Some(typ.clone()),
                    PyObject::ExceptionGroup { typ, .. } => Some(typ.clone()),
                    _ => None,
                };
                let matched = match typ_name {
                    Some(t) => exc_type_matches(&expected, &t)?,
                    None => false,
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
                    exc
                } else {
                    match self.frames[fi].pop() {
                        Ok(exc) => exc,
                        Err(_) => return Err(PyError::runtime_error("No active exception to re-raise")),
                    }
                };
                // Check if it's an empty ExceptionGroup (all exceptions were handled)
                let is_empty_eg = match &*reraise_exc.borrow() {
                    PyObject::ExceptionGroup { exceptions, .. } => exceptions.is_empty(),
                    _ => false,
                };
                if !is_empty_eg {
                    return Err(PyError::Exception("re-raise".to_string(), reraise_exc));
                }
                // Empty group — all exceptions handled, silently continue
            }

            Opcode::RAISE_VARARGS => {
                let nargs = arg;
                match nargs {
                    0 => {
                        // Bare raise: re-raise the current exception from the stack
                        match self.frames[fi].stack.pop() {
                            Some(exc) => {
                                return Err(PyError::Exception(format!("re-raise"), exc));
                            }
                            None => return Err(PyError::runtime_error("No active exception to re-raise")),
                        }
                    }
                    1 => {
                        let exc = self.frames[fi].pop()?;
                        // If the raised value is already an exception instance, use it directly
                        let is_callable = !matches!(&*exc.borrow(), 
                            PyObject::Str(_) | PyObject::Exception { .. } | PyObject::ExceptionGroup { .. } | PyObject::Instance { .. }
                        );
                        let exc = if is_callable {
                            let exc_clone = exc.clone();
                            match self.call_function(exc_clone, vec![], vec![]) {
                                Ok(instance) => instance,
                                Err(_) => return Err(PyError::type_error("exceptions must be str or Exception instances")),
                            }
                        } else {
                            exc
                        };
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
                                let args = dict.get("args");
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
                        // Store exc_info before returning error
                        self.exc_type = Some(exc.clone());
                        self.exc_value = Some(exc.clone());
                        self.exc_traceback = Some(py_none());
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
                        // Set __cause__ on the exception object (cause field changed to PyObjectRef)
                        if let PyObject::Exception { cause: ref mut cause_field, .. } = &mut *exc.borrow_mut() {
                            *cause_field = Some(cause.clone());
                        }
                        return Err(PyError::Exception(format!("{} (caused by: {})", exc_msg, cause_msg), exc));
                    }
                    _ => return Err(PyError::runtime_error("invalid RAISE_VARARGS count")),
                }
            }

            Opcode::IMPORT_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
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
                            .get("__package__").cloned()
                            .and_then(|p| {
                                let p = p.borrow();
                                if let PyObject::Str(s) = &*p { Some(s.to_string()) } else { None }
                            });
                        let resolved_name = match pkg {
                            Some(p) if !p.is_empty() => {
                                if name.is_empty() { p } else { format!("{}.{}", p, name) }
                            }
                            // Fallback: use __name__ up to last dot as package
                            _ => {
                                let n = self.frames[fi].globals.borrow()
                                    .get("__name__").cloned()
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
                if let Some(module) = self.modules.get(&resolved) {
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
                                    if let Some(md) = dict.get("modules").cloned() {
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
                        Err(msg) => return Err(PyError::ImportError(msg)),
                    }
                }
            }

            Opcode::IMPORT_FROM => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let module = self.frames[fi].peek(0)?;
                // Handle 'from module import *' — when the imported name is '*',
                // iterate over the module's dict and store all names in current scope
                if name == "*" {
                    let module_borrowed = module.borrow();
                    if let PyObject::Module { dict, .. } = &*module_borrowed {
                        // Use __all__ if present, otherwise all non-underscore names
                        let names_to_import: Vec<String> = if let Some(all_val) = dict.get("__all__") {
                            let all_borrowed = all_val.borrow();
                            match &*all_borrowed {
                                PyObject::Tuple(items) | PyObject::List(items) => {
                                    items.iter().filter_map(|n| {
                                        if let PyObject::Str(s) = &*n.borrow() { Some(s.to_string()) } else { None }
                                    }).collect()
                                }
                                _ => dict.keys().filter(|k| !k.starts_with('_')).cloned().collect(),
                            }
                        } else {
                            dict.keys().filter(|k| !k.starts_with('_')).cloned().collect()
                        };
                        // Collect name-value pairs before dropping borrow
                        let imports: Vec<(String, PyObjectRef)> = names_to_import.iter()
                            .filter_map(|name| dict.get_str(&name).map(|val| (name.clone(), val.clone())))
                            .collect();
                        drop(module_borrowed);
                        for (import_name, val) in &imports {
                            self.frames[fi].globals.borrow_mut().insert(import_name.clone(), val.clone());
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
                let found = found.or_else(|| {
                    self.frames.iter().find_map(|f| {
                        let g = f.globals.borrow();
                        if g.get("__name__").map(|n| n.str()).as_deref() == Some(module_name.as_str()) {
                            g.get(&name).cloned()
                        } else {
                            None
                        }
                    })
                });
                if let Some(val) = found {
                    self.frames[fi].push(val);
                } else {
                    // Try importing as sub-module (for dotted names like os.path)
                    let submodule_name = format!("{}.{}", module_name, name);
                    if submodule_name.contains('.') {
                        match self.import_module_from_file(&submodule_name) {
                            Ok(submod) => {
                                self.modules.insert(submodule_name.clone(), submod.clone());
                                if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                                    dict.insert_str(&name, submod.clone());
                                }
                                self.frames[fi].push(submod);
                            }
                            Err(_) => {
                                return Err(PyError::ImportError(format!("cannot import name '{}' from '{}'", name, module_name)));
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
                        if let Some(var_idx) = f.code.varnames.iter().position(|n| n == name) {
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
                // Call manager.__exit__(exc_type, exc_val, traceback)
                let mgr = self.frames[fi].pop()?;
                let (typ_str, val) = {
                    let exc = self.frames[fi].peek(0)?;
                    let exc_borrowed = exc.borrow();
                    match &*exc_borrowed {
                        PyObject::Exception { typ, args, .. } => {
                            (py_str(typ), args.first().cloned().unwrap_or_else(|| py_none()))
                        }
                        _ => (py_str("Exception"), py_none()),
                    }
                };
                let exit_method = mgr.borrow().get_attribute("__exit__")
                    .map_err(|_| PyError::attribute_error("context manager has no __exit__"))?;
                // Bind the manager as self so __exit__ can access it
                let bound = PyObjectRef::imm(PyObject::BoundMethod {
                    func: exit_method,
                    self_obj: mgr,
                });
                let result = self.call_function(bound, vec![typ_str, val, py_none()], vec![])?;
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
                        frame: std::cell::RefCell::new(Some(frame)),
                    });
                    return Ok(Some(gen));
                } else {
                    let gen = PyObjectRef::new(PyObject::Generator {
                        frame: std::cell::RefCell::new(Some(frame)),
                    });
                    return Ok(Some(gen));
                }
            }

            Opcode::GET_AWAITABLE => {
                // Call __await__ on the object to get an iterator
                let obj = self.frames[fi].pop()?;
                let await_method = obj.borrow().get_attribute("__await__")
                    .map_err(|_| PyError::type_error("object does not support __await__"))?;
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

    pub(crate) fn call_function(&mut self, callable: PyObjectRef, args: Vec<PyObjectRef>, keywords: Vec<(String, PyObjectRef)>) -> PyResult<PyObjectRef> {
        let type_name = callable.borrow().type_name();
        if cfg!(feature = "profile") { eprintln!("DEBUG call_function: type={} name={:?}", type_name, callable.repr()); }

        if let PyObject::BuiltinFunction { func, .. } = &*callable.borrow() {
            let func = *func;
            // Pack keyword arguments into a dict and append as last arg
            if !keywords.is_empty() {
                let mut dict = crate::object::PyDict::new();
                for (k, v) in &keywords {
                    let _ = dict.set(crate::object::py_str(k), v.clone());
                }
                let mut new_args = args;
                new_args.push(crate::object::PyObjectRef::new(crate::object::PyObject::Dict(dict)));
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
                new_args.push(crate::object::PyObjectRef::new(crate::object::PyObject::Dict(dict)));
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
        if let PyObject::Function { code, globals: func_globals, defaults, closure, jit_ptr, jit_consts, .. } = &*callable.borrow() {
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
            let func_globals = func_globals.clone();
            let defaults = defaults.clone();
            let code_rc = Rc::new(code.clone());
            let mut new_frame = self.acquire_frame(Rc::clone(&code_rc), func_globals, Rc::clone(&self.builtins), None);
            new_frame.closure = closure.clone();
            let code = code;

            let npos = args.len();
            let named_params = code.arg_count;

            // Assign positional args to named parameters
            for i in 0..npos.min(named_params) {
                let name_clone = new_frame.code.varnames[i].clone();
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
                if let Some(idx) = new_frame.code.varnames.iter().position(|n| n == vararg_name) {
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
                        let name_clone = new_frame.code.varnames[i].clone();
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
            if let Some(kwarg_name) = &code.kwarg_name {
                let kw_dict = py_dict();
                for (key, value) in &keywords {
                    if let Some(idx) = new_frame.code.varnames.iter().position(|n| n == key) {
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
                if let Some(idx) = new_frame.code.varnames.iter().position(|n| n == kwarg_name) {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(kw_dict.clone());
                    }
                }
                new_frame.insert_local(&kwarg_name, kw_dict);
            } else {
                // No **kwargs: keyword args must still bind to the matching
                // named parameter's FAST local slot (LOAD_FAST reads
                // fast_locals, not the insert_local name dict — missing this
                // meant `f(1, somekw=True)` left `somekw` as None in
                // fast_locals, raising "referenced before assignment" the
                // moment the function body read it), matching the
                // **kwargs branch above.
                for (key, value) in &keywords {
                    if let Some(idx) = new_frame.code.varnames.iter().position(|n| n == key) {
                        if idx < new_frame.fast_locals.len() {
                            new_frame.fast_locals[idx] = Some(value.clone());
                        }
                    }
                    new_frame.insert_local(&key, value.clone());
                }
            }

            self.frames.push(new_frame);
            let result = self.execute();
            if let Some(frame) = self.frames.pop() {
                self.release_frame(frame);
            }
            return result;
        }

        if let PyObject::Type { dict, mro, .. } = &*callable.borrow() {
            let native_kind = dict.get_str(crate::object::NATIVE_BASE_MARKER).map(|v| v.str());
            let mut instance_dict = HashMap::new();
            if let Some(kind) = &native_kind {
                instance_dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), crate::object::make_native_backing(kind));
            }
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: callable.clone(),
                dict: instance_dict,
            });
            let init_func = dict.get("__init__").cloned().or_else(|| {
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
            if init_func.is_none() {
                // No Python- or Rust-defined __init__ anywhere in the mro:
                // for a native-subclassing class (`class Foo(list): pass`),
                // that means the constructor call itself must behave like
                // list(iterable)/dict(...)/str(x).
                if let Some(kind) = &native_kind {
                    let native = crate::object::synthesize_native_init(kind, &args)?;
                    if let PyObject::Instance { dict, .. } = &mut *instance.borrow_mut() {
                        dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), native);
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

            let namespace = Rc::new(RefCell::new(HashMap::new()));

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
                PyObject::Function { code, closure, .. } => {
                    let code = code.clone();
                    let closure = closure.clone();
                    let mut new_frame = self.acquire_frame(Rc::new(code), namespace.clone(), Rc::clone(&self.builtins), caller_module_globals);
                    new_frame.closure = closure;
                    self.frames.push(new_frame);
                    self.execute()?;
                    if let Some(frame) = self.frames.pop() {
                        self.release_frame(frame);
                    }
                }
                _ => return Err(PyError::type_error("class body must be a function")),
            }

            let namespace_dict = namespace.borrow().clone();

            // Extract metaclass from keyword arguments (if any)
            let metaclass = keywords.iter()
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
                let object_type = self.builtins.get("object").cloned()
                    .unwrap_or_else(|| {
                        // Fallback: create a minimal object type
                        let mut obj_dict = HashMap::new();
                        obj_dict.insert("__setattr__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "__setattr__".to_string(),
                            func: |args| {
                                if args.len() < 3 { return Err(PyError::type_error("__setattr__ needs 3 args")); }
                                args[0].borrow_mut().set_attribute(&args[1].str(), args[2].clone())?;
                                Ok(py_none())
                            },
                        }));
                        PyObjectRef::new(PyObject::Type {
                            name: "object".to_string(),
                            dict: obj_dict,
                            bases: vec![],
                            mro: vec![],
                        })
                    });
                vec![object_type]
            } else {
                bases_vec
            };

            // If a metaclass was specified, delegate class creation to it
            if let Some(mc) = metaclass {
                // Build a PyDict from the namespace HashMap for the metaclass call
                let namespace_py_dict = {
                    let mut pd = PyDict::new();
                    for (k, v) in &namespace_dict {
                        pd.set(py_str(k), v.clone())?;
                    }
                    PyObjectRef::new(PyObject::Dict(pd))
                };
                let bases_tuple = PyObjectRef::imm(PyObject::Tuple(bases_vec));
                let mc_result = self.call_function(
                    mc,
                    vec![name.clone(), bases_tuple, namespace_py_dict],
                    vec![],
                )?;
                return Ok(mc_result);
            }

            let mut namespace_dict = namespace_dict;
            // Detect `class Foo(list): ...` / `(dict)` / `(str)` — either a
            // direct native base, or inherited transitively through a base
            // that already carries the marker (propagated down so every
            // subclass's own dict has it, without needing to walk mro/bases
            // again at instantiation or dispatch time).
            for base in &bases_vec {
                let native_name = match &*base.borrow() {
                    PyObject::BuiltinFunction { name, .. } if crate::object::is_recognized_native_base_name(name) => Some(name.clone()),
                    _ => crate::object::native_base_of_type(base),
                };
                if let Some(native_name) = native_name {
                    namespace_dict.insert(crate::object::NATIVE_BASE_MARKER.to_string(), py_str(&native_name));
                    break;
                }
            }

            let class = PyObjectRef::new(PyObject::Type {
                name: name_str,
                dict: namespace_dict.clone(),
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

            // __init_subclass__ protocol: call on each base class with non-metaclass kwargs
            let init_subclass_kwargs: Vec<(String, PyObjectRef)> = keywords.iter()
                .filter(|(k, _)| k != "metaclass")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for base in &bases_vec {
                if let Ok(init_subclass) = base.borrow().get_attribute("__init_subclass__") {
                    let _ = self.call_function(init_subclass, vec![class.clone()], init_subclass_kwargs.clone());
                }
            }

            return Ok(class);
        }

        if let PyObject::Closure(c) = &*callable.borrow() {
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
            let mut call_args = vec![callable.clone()];
            call_args.extend(args.iter().cloned());
            return self.call_function(f, call_args, keywords);
        }

        Err(PyError::type_error(format!("'{}' object is not callable", type_name)))
    }

    fn handle_exception(&mut self, error: &PyError) -> bool {
        // Search all frames for exception handlers
        for frame in self.frames.iter_mut().rev() {
            while let Some(handler) = frame.exception_handlers.pop() {
                // For any handler (Except or Finally), restore stack and transfer control
                frame.stack.truncate(handler.stack_depth);
                frame.ip = handler.instr_addr;
                let exc_obj = {
                    let (typ, cause, _is_group) = match error {
                        PyError::TypeError(_) => ("TypeError".to_string(), None, false),
                        PyError::ValueError(_) => ("ValueError".to_string(), None, false),
                        PyError::NameError(_) => ("NameError".to_string(), None, false),
                        PyError::AttributeError(_) => ("AttributeError".to_string(), None, false),
                        PyError::IndexError(_) => ("IndexError".to_string(), None, false),
                        PyError::KeyError(_) => ("KeyError".to_string(), None, false),
                        PyError::ZeroDivisionError(_) => ("ZeroDivisionError".to_string(), None, false),
                        PyError::RuntimeError(_) => ("RuntimeError".to_string(), None, false),
                        PyError::StopIteration => ("StopIteration".to_string(), None, false),
                        PyError::ImportError(_) => ("ImportError".to_string(), None, false),
                        PyError::Exception(_, exc) => {
                            let exc_borrow = exc.borrow();
                            match &*exc_borrow {
                                PyObject::Exception { typ, cause, .. } => (typ.clone(), cause.clone(), false),
                                PyObject::ExceptionGroup { .. } => {
                                    // Preserve ExceptionGroup: push it directly
                                    drop(exc_borrow);
                                    frame.push(exc.clone());
                                    return true;
                                }
                                _ => ("Exception".to_string(), None, false),
                            }
                        }
                        _ => ("Exception".to_string(), None, false),
                    };
                    PyObjectRef::imm(PyObject::Exception {
                        typ,
                        args: vec![py_str(&error.message())],
                        cause,
                    })
                };
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
                Err(e) if matches!(&e, PyError::StopIteration) => {
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
        PyObject::Str(s) => Ok(is_exception_subclass(exc_type_name, s)),
        PyObject::Type { name, .. } => Ok(is_exception_subclass(exc_type_name, name)),
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

fn is_exception_subclass(child_type: &str, parent_type: &str) -> bool {
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
        // Children of ValueError
        "UnicodeError" | "UnicodeEncodeError" | "UnicodeDecodeError" |
        "UnicodeTranslateError" => Some("ValueError"),
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
fn format_with_spec(val: &PyObjectRef, spec_str: &str) -> PyResult<String> {
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
            Some(chars[start..idx].iter().collect::<String>().parse::<usize>().unwrap())
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
            Some(chars[start..idx].iter().collect::<String>().parse::<usize>().unwrap())
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
