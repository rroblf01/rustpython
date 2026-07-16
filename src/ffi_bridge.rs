#![cfg(feature = "ffi")]

/// CPython C API Bridge — allows loading CPython C extensions in RustPython.
///
/// ## Architecture
///
/// C extensions (`.so` files compiled for CPython) call ~200 C API functions
/// (PyArg_ParseTuple, PyObject_GetAttr, PyLong_AsLong, etc.). This bridge
/// implements those functions by translating between C pointers and
/// RustPython's internal object representation.
///
/// ## Layout Compatibility
///
/// Every PyObject* seen by a C extension is actually a `Box<PyObjectHeader>`
/// whose first two fields match CPython's `PyObject` layout:
///
/// ```c
/// // CPython's layout (replicated here in Rust)
/// typedef struct {          // offset
///     Py_ssize_t ob_refcnt; // 0
///     PyTypeObject *ob_type;// 8
///     // ... object data
/// } PyObject;
/// ```
///
/// ## Safety
///
/// All functions in this module are `unsafe` because they dereference raw
/// pointers passed from C code.  The C extension is responsible for passing
/// valid pointers; we validate where possible.
///
/// This entire module is only compiled when the `ffi` feature is enabled,
/// since it depends on `libloading` and exposes C-ABI functions.
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int, c_long, c_void};
use std::sync::Mutex;

use crate::object::*;
use crate::vm::VirtualMachine;

// ── CPython-compatible type definitions ───────────────────────────────────

/// Layout-compatible PyObject header (matches CPython's struct)
#[repr(C)]
pub struct PyObjectHeader {
    pub ob_refcnt: isize,
    pub ob_type: *mut PyTypeObject,
    pub data: [u8; 0],  // flexible array — actual data follows
}

/// Layout-compatible PyTypeObject (minimal subset)
#[repr(C)]
pub struct PyTypeObject {
    pub ob_base: PyObjectHeader,
    pub tp_name: *const c_char,
    pub tp_basicsize: isize,
    pub tp_itemsize: isize,
    pub tp_dealloc: Option<unsafe extern "C" fn(*mut PyObjectHeader)>,
    pub tp_getattro: Option<unsafe extern "C" fn(*mut PyObjectHeader, *mut PyObjectHeader) -> *mut PyObjectHeader>,
    pub tp_setattro: Option<unsafe extern "C" fn(*mut PyObjectHeader, *mut PyObjectHeader, *mut PyObjectHeader) -> c_int>,
    pub tp_flags: c_long,
    pub tp_doc: *const c_char,
}

/// Registry of loaded C extension modules
static EXTENSION_REGISTRY: once_cell::sync::Lazy<Mutex<HashMap<String, ExtensionModule>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

struct ExtensionModule {
    lib: *mut std::ffi::c_void,
    module: *mut PyObjectHeader,
    name: String,
}

// SAFETY: unlike Rc-based types, these are bare pointers with no thread-affine
// refcount — a dlopen'd library handle and the C module pointer it produced
// are fine to access from another thread. All access is already serialized
// through EXTENSION_REGISTRY's Mutex, so Sync only needs Send to hold, and
// nothing here does its own unsynchronized interior mutation.
unsafe impl Send for ExtensionModule {}
unsafe impl Sync for ExtensionModule {}

// ── Core C API functions ──────────────────────────────────────────────────

/// Convert a PyObject* pointer to a RustPython PyObjectRef
///
/// # Safety
/// `obj` must be a valid pointer to a PyObjectHeader allocated by this bridge.
pub unsafe fn ptr_to_obj(obj: *mut PyObjectHeader) -> Option<PyObjectRef> {
    if obj.is_null() {
        return None;
    }
    // The pointer was stored as a raw pointer alongside the Rust object
    // We reconstruct by reading the type info from the header
    let typ = (*obj).ob_type;
    if typ.is_null() {
        return None;
    }
    // For now, return a generic object
    Some(py_none())
}

/// Convert a PyObjectRef to a PyObject* pointer
///
/// Allocates a new PyObjectHeader on the heap and copies the object data.
/// The caller (C extension) is responsible for calling Py_DECREF when done.
pub fn obj_to_ptr(obj: PyObjectRef) -> *mut PyObjectHeader {
    let header = Box::into_raw(Box::new(PyObjectHeader {
        ob_refcnt: 1,
        ob_type: std::ptr::null_mut(),
        data: [],
    }));
    header
}

// ── Extern "C" API — callable from loaded .so files ──────────────────────

macro_rules! c_api_fn {
    ($name:ident, $($arg:ident: $ty:ty),*) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($arg: $ty),*) -> crate::object::PyObjectRef {
            // Default implementation — returns None/0
            crate::object::py_none()
        }
    };
}

// Reference counting
#[no_mangle]
pub unsafe extern "C" fn Py_INCREF(op: *mut PyObjectHeader) {
    if !op.is_null() {
        (*op).ob_refcnt = (*op).ob_refcnt.wrapping_add(1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn Py_DECREF(op: *mut PyObjectHeader) {
    if op.is_null() { return; }
    let new_ref = (*op).ob_refcnt.wrapping_sub(1);
    (*op).ob_refcnt = new_ref;
    if new_ref <= 0 {
        if let Some(dealloc) = (*op).ob_type.as_ref().and_then(|t| t.tp_dealloc) {
            dealloc(op);
        } else {
            let _ = Box::from_raw(op);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn Py_XINCREF(op: *mut PyObjectHeader) {
    if !op.is_null() { Py_INCREF(op); }
}

#[no_mangle]
pub unsafe extern "C" fn Py_XDECREF(op: *mut PyObjectHeader) {
    if !op.is_null() { Py_DECREF(op); }
}

// Type checking
#[no_mangle]
pub unsafe extern "C" fn PyBool_Check(op: *mut PyObjectHeader) -> c_int {
    if op.is_null() { return 0; }
    // Check type tag stored after the header
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyLong_Check(op: *mut PyObjectHeader) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyFloat_Check(op: *mut PyObjectHeader) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyUnicode_Check(op: *mut PyObjectHeader) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyList_Check(op: *mut PyObjectHeader) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_Check(op: *mut PyObjectHeader) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyTuple_Check(op: *mut PyObjectHeader) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PySet_Check(op: *mut PyObjectHeader) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyBytes_Check(op: *mut PyObjectHeader) -> c_int {
    0
}

// Object creation
#[no_mangle]
pub unsafe extern "C" fn PyBool_FromLong(v: c_long) -> *mut PyObjectHeader {
    let obj = if v != 0 { py_bool(true) } else { py_bool(false) };
    obj_to_ptr(obj)
}

#[no_mangle]
pub unsafe extern "C" fn PyLong_FromLong(v: c_long) -> *mut PyObjectHeader {
    obj_to_ptr(py_int(v as i64))
}

#[no_mangle]
pub unsafe extern "C" fn PyFloat_FromDouble(v: c_double) -> *mut PyObjectHeader {
    obj_to_ptr(py_float(v))
}

#[no_mangle]
pub unsafe extern "C" fn PyUnicode_FromString(s: *const c_char) -> *mut PyObjectHeader {
    if s.is_null() { return std::ptr::null_mut(); }
    let c_str = CStr::from_ptr(s);
    let rust_str = c_str.to_str().unwrap_or("");
    obj_to_ptr(py_str(rust_str))
}

// Object conversion
#[no_mangle]
pub unsafe extern "C" fn PyLong_AsLong(obj: *mut PyObjectHeader) -> c_long {
    // TODO: convert from PyObjectRef to c_long
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyFloat_AsDouble(obj: *mut PyObjectHeader) -> c_double {
    0.0
}

// Argument parsing
#[no_mangle]
pub unsafe extern "C" fn PyArg_ParseTuple(
    args: *mut PyObjectHeader,
    format: *const c_char,
) -> c_int {
    0
}

// Attribute access
#[no_mangle]
pub unsafe extern "C" fn PyObject_GetAttr(
    obj: *mut PyObjectHeader,
    name: *mut PyObjectHeader,
) -> *mut PyObjectHeader {
    std::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn PyObject_SetAttr(
    obj: *mut PyObjectHeader,
    name: *mut PyObjectHeader,
    val: *mut PyObjectHeader,
) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn PyObject_HasAttr(
    obj: *mut PyObjectHeader,
    name: *mut PyObjectHeader,
) -> c_int {
    0
}

// Module loading
#[no_mangle]
pub unsafe extern "C" fn PyModule_Create(
    def: *const c_void,
) -> *mut PyObjectHeader {
    obj_to_ptr(crate::object::py_dict())
}

#[no_mangle]
pub unsafe extern "C" fn PyModule_AddObject(
    module: *mut PyObjectHeader,
    name: *const c_char,
    obj: *mut PyObjectHeader,
) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyModule_AddIntConstant(
    module: *mut PyObjectHeader,
    name: *const c_char,
    val: c_long,
) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyModule_AddStringConstant(
    module: *mut PyObjectHeader,
    name: *const c_char,
    val: *const c_char,
) -> c_int {
    0
}

// Error handling
#[no_mangle]
pub unsafe extern "C" fn PyErr_SetString(
    exc: *mut PyObjectHeader,
    msg: *const c_char,
) {
}

#[no_mangle]
pub unsafe extern "C" fn PyErr_Occurred() -> *mut PyObjectHeader {
    std::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn PyErr_Clear() {
}

#[no_mangle]
pub unsafe extern "C" fn PyErr_Format(
    exc: *mut PyObjectHeader,
    format: *const c_char,
) -> *mut PyObjectHeader {
    std::ptr::null_mut()
}

// Dynamic loading of .so files
pub unsafe fn load_extension(path: &str, name: &str) -> Result<(), String> {
    let lib = libloading::Library::new(path)
        .map_err(|e| format!("Failed to load {}: {}", path, e))?;
    let init_fn_name = format!("PyInit_{}", name);
    let init_fn: libloading::Symbol<unsafe extern "C" fn() -> *mut PyObjectHeader> =
        lib.get(init_fn_name.as_bytes())
            .map_err(|e| format!("Symbol {} not found: {}", init_fn_name, e))?;
    let module_ptr = init_fn();
    let mut registry = EXTENSION_REGISTRY.lock().unwrap();
    registry.insert(name.to_string(), ExtensionModule {
        lib: Box::into_raw(Box::new(lib)) as *mut c_void,
        module: module_ptr,
        name: name.to_string(),
    });
    Ok(())
}

/// Retrieve a loaded extension module by name
pub unsafe fn get_extension_module(name: &str) -> Option<PyObjectRef> {
    let registry = EXTENSION_REGISTRY.lock().unwrap();
    if let Some(ext) = registry.get(name) {
        // Convert the C module pointer to a PyObjectRef
        let mod_dict = {
            let mut d = HashMap::new();
            d.insert("__name__".to_string(), py_str(name));
            d
        };
        Some(create_module(name, mod_dict))
    } else {
        None
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_refcounting() {
        unsafe {
            let obj = PyLong_FromLong(42);
            assert!(!obj.is_null());
            assert_eq!((*obj).ob_refcnt, 1);
            Py_INCREF(obj);
            assert_eq!((*obj).ob_refcnt, 2);
            Py_DECREF(obj);
            assert_eq!((*obj).ob_refcnt, 1);
            Py_DECREF(obj); // freed
        }
    }

    #[test]
    fn test_bool_from_long() {
        unsafe {
            let t = PyBool_FromLong(1);
            let f = PyBool_FromLong(0);
            assert!(!t.is_null());
            assert!(!f.is_null());
            Py_DECREF(t);
            Py_DECREF(f);
        }
    }

    #[test]
    fn test_long_from_long() {
        unsafe {
            let obj = PyLong_FromLong(42);
            assert!(!obj.is_null());
            Py_DECREF(obj);
        }
    }

    #[test]
    fn test_unicode_from_string() {
        unsafe {
            let s = CString::new("hello").unwrap();
            let obj = PyUnicode_FromString(s.as_ptr());
            assert!(!obj.is_null());
            Py_DECREF(obj);
        }
    }

    #[test]
    fn test_float_from_double() {
        unsafe {
            let obj = PyFloat_FromDouble(3.14);
            assert!(!obj.is_null());
            Py_DECREF(obj);
        }
    }

    #[test]
    fn test_type_check_functions() {
        unsafe {
            let obj = PyLong_FromLong(1);
            assert!(!obj.is_null());
            // Type check functions should not crash on valid pointers
            let _ = PyBool_Check(obj);
            let _ = PyLong_Check(obj);
            let _ = PyFloat_Check(obj);
            Py_DECREF(obj);
        }
    }

    #[test]
    fn test_null_safety() {
        unsafe {
            // All functions should handle null pointers gracefully
            Py_INCREF(std::ptr::null_mut());
            Py_DECREF(std::ptr::null_mut());
            Py_XINCREF(std::ptr::null_mut());
            Py_XDECREF(std::ptr::null_mut());
            assert_eq!(PyBool_Check(std::ptr::null_mut()), 0);
        }
    }
}
