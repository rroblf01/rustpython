use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;


/// Native __future__ module: defines _Feature tuples and feature flags.
pub fn create_future_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // _Feature helper: tuples of (flag, name, first_release, optional_since)
    let feature = |flag: i64, name: &str, first: &str, optional: &str| -> PyObjectRef {
        PyObjectRef::imm(PyObject::Tuple(vec![
            py_int(flag),
            py_str(name),
            py_str(first),
            py_str(optional),
        ]))
    };

    d.insert_str(
        "nested_scopes",
        feature(0x01, "nested_scopes", "2.1.0", "2.2.0"),
    );
    d.insert_str("generators", feature(0x02, "generators", "2.2.0", "2.3.0"));
    d.insert_str("division", feature(0x04, "division", "2.2.0", "3.0.0"));
    d.insert_str(
        "absolute_import",
        feature(0x08, "absolute_import", "2.5.0", "3.0.0"),
    );
    d.insert_str(
        "with_statement",
        feature(0x10, "with_statement", "2.5.0", "2.6.0"),
    );
    d.insert_str(
        "print_function",
        feature(0x20, "print_function", "2.6.0", "3.0.0"),
    );
    d.insert_str(
        "unicode_literals",
        feature(0x40, "unicode_literals", "2.6.0", "3.0.0"),
    );
    d.insert_str(
        "barry_as_FLUFL",
        feature(0x80, "barry_as_FLUFL", "3.1.0", "4.0.0"),
    );
    d.insert_str(
        "generator_stop",
        feature(0x100, "generator_stop", "3.5.0", "3.7.0"),
    );
    d.insert_str(
        "annotations",
        feature(0x200, "annotations", "3.7.0", "3.11.0"),
    );

    // CO_FUTURE_* constants (real CPython's __future__.py defines these as
    // `flags` values usable with compile()) — test_flufl references
    // CO_FUTURE_BARRY_AS_BDFL.
    d.insert_str("CO_FUTURE_DIVISION", py_int(0x20000));
    d.insert_str("CO_FUTURE_ABSOLUTE_IMPORT", py_int(0x40000));
    d.insert_str("CO_FUTURE_WITH_STATEMENT", py_int(0x80000));
    d.insert_str("CO_FUTURE_PRINT_FUNCTION", py_int(0x100000));
    d.insert_str("CO_FUTURE_UNICODE_LITERALS", py_int(0x200000));
    d.insert_str("CO_FUTURE_BARRY_AS_BDFL", py_int(0x400000));
    d.insert_str("CO_FUTURE_GENERATOR_STOP", py_int(0x800000));
    d.insert_str("CO_FUTURE_ANNOTATIONS", py_int(0x1000000));

    d.insert_str(
        "all_feature_names",
        py_list(vec![
            py_str("nested_scopes"),
            py_str("generators"),
            py_str("division"),
            py_str("absolute_import"),
            py_str("with_statement"),
            py_str("print_function"),
            py_str("unicode_literals"),
            py_str("barry_as_FLUFL"),
            py_str("generator_stop"),
            py_str("annotations"),
        ]),
    );

    d.insert_str(
        "__doc__",
        py_str("Future feature statements (from __future__)"),
    );
    d.insert_str("__name__", py_str("__future__"));
    d.insert_str("__package__", py_str(""));
    d
}

/// Native errno module — POSIX error code constants
pub fn create_errno_dict() -> HashMap<String, PyObjectRef> {
    let mut d: HashMap<String, PyObjectRef> = HashMap::new();
    // Standard POSIX errno codes used by tempfile and os modules
    d.insert_str("EPERM", py_int(1));
    d.insert_str("ENOENT", py_int(2));
    d.insert_str("ESRCH", py_int(3));
    d.insert_str("EINTR", py_int(4));
    d.insert_str("EIO", py_int(5));
    d.insert_str("ENXIO", py_int(6));
    d.insert_str("E2BIG", py_int(7));
    d.insert_str("ENOEXEC", py_int(8));
    d.insert_str("EBADF", py_int(9));
    d.insert_str("ECHILD", py_int(10));
    d.insert_str("EAGAIN", py_int(11));
    d.insert_str("ENOMEM", py_int(12));
    d.insert_str("EACCES", py_int(13));
    d.insert_str("EFAULT", py_int(14));
    d.insert_str("ENOTBLK", py_int(15));
    d.insert_str("EBUSY", py_int(16));
    d.insert_str("EEXIST", py_int(17));
    d.insert_str("EXDEV", py_int(18));
    d.insert_str("ENODEV", py_int(19));
    d.insert_str("ENOTDIR", py_int(20));
    d.insert_str("EISDIR", py_int(21));
    d.insert_str("EINVAL", py_int(22));
    d.insert_str("ENFILE", py_int(23));
    d.insert_str("EMFILE", py_int(24));
    d.insert_str("ENOTTY", py_int(25));
    d.insert_str("ETXTBSY", py_int(26));
    d.insert_str("EFBIG", py_int(27));
    d.insert_str("ENOSPC", py_int(28));
    d.insert_str("ESPIPE", py_int(29));
    d.insert_str("EROFS", py_int(30));
    d.insert_str("EMLINK", py_int(31));
    d.insert_str("EPIPE", py_int(32));
    d.insert_str("EDOM", py_int(33));
    d.insert_str("ERANGE", py_int(34));
    d.insert_str("ENOSYS", py_int(38));
    d.insert_str("EOPNOTSUPP", py_int(95));
    d.insert_str("ENOTSUP", py_int(95));
    d.insert_str("ENOTSOCK", py_int(88));
    d.insert_str("ECONNABORTED", py_int(103));
    d.insert_str("ENOTCONN", py_int(107));
    d.insert_str("EALREADY", py_int(114));
    d.insert_str("EADDRINUSE", py_int(98));
    d.insert_str("EADDRNOTAVAIL", py_int(99));
    d.insert_str("EADV", py_int(68));
    d.insert_str("EAFNOSUPPORT", py_int(97));
    d.insert_str("EBADE", py_int(52));
    d.insert_str("EBADFD", py_int(77));
    d.insert_str("EBADMSG", py_int(74));
    d.insert_str("EBADR", py_int(53));
    d.insert_str("EBADRQC", py_int(56));
    d.insert_str("EBADSLT", py_int(57));
    d.insert_str("EBFONT", py_int(59));
    d.insert_str("ECANCELED", py_int(125));
    d.insert_str("ECHRNG", py_int(44));
    d.insert_str("ECOMM", py_int(70));
    d.insert_str("ECONNREFUSED", py_int(111));
    d.insert_str("ECONNRESET", py_int(104));
    d.insert_str("EDEADLK", py_int(35));
    d.insert_str("EDEADLOCK", py_int(35));
    d.insert_str("EDESTADDRREQ", py_int(89));
    d.insert_str("EDOTDOT", py_int(73));
    d.insert_str("EDQUOT", py_int(122));
    d.insert_str("EHOSTDOWN", py_int(112));
    d.insert_str("EHOSTUNREACH", py_int(113));
    d.insert_str("EHWPOISON", py_int(133));
    d.insert_str("EIDRM", py_int(43));
    d.insert_str("EILSEQ", py_int(84));
    d.insert_str("EISNAM", py_int(120));
    d.insert_str("EKEYEXPIRED", py_int(127));
    d.insert_str("EKEYREJECTED", py_int(129));
    d.insert_str("EKEYREVOKED", py_int(128));
    d.insert_str("EL2HLT", py_int(51));
    d.insert_str("EL2NSYNC", py_int(45));
    d.insert_str("EL3HLT", py_int(46));
    d.insert_str("EL3RST", py_int(47));
    d.insert_str("ELIBACC", py_int(79));
    d.insert_str("ELIBBAD", py_int(80));
    d.insert_str("ELIBEXEC", py_int(83));
    d.insert_str("ELIBMAX", py_int(82));
    d.insert_str("ELIBSCN", py_int(81));
    d.insert_str("ELNRNG", py_int(48));
    d.insert_str("ELOOP", py_int(40));
    d.insert_str("EMEDIUMTYPE", py_int(124));
    d.insert_str("EMSGSIZE", py_int(90));
    d.insert_str("EMULTIHOP", py_int(72));
    d.insert_str("ENAMETOOLONG", py_int(36));
    d.insert_str("ENAVAIL", py_int(119));
    d.insert_str("ENETDOWN", py_int(100));
    d.insert_str("ENETRESET", py_int(102));
    d.insert_str("ENETUNREACH", py_int(101));
    d.insert_str("ENOANO", py_int(55));
    d.insert_str("ENOBUFS", py_int(105));
    d.insert_str("ENOCSI", py_int(50));
    d.insert_str("ENODATA", py_int(61));
    d.insert_str("ENOKEY", py_int(126));
    d.insert_str("ENOLCK", py_int(37));
    d.insert_str("ENOLINK", py_int(67));
    d.insert_str("ENOMEDIUM", py_int(123));
    d.insert_str("ENOMSG", py_int(42));
    d.insert_str("ENONET", py_int(64));
    d.insert_str("ENOPKG", py_int(65));
    d.insert_str("ENOSR", py_int(63));
    d.insert_str("ENOSTR", py_int(60));
    d.insert_str("ENOTEMPTY", py_int(39));
    d.insert_str("ENOTNAM", py_int(118));
    d.insert_str("ENOTRECOVERABLE", py_int(131));
    d.insert_str("ENOTUNIQ", py_int(76));
    d.insert_str("EOVERFLOW", py_int(75));
    d.insert_str("EOWNERDEAD", py_int(130));
    d.insert_str("EPFNOSUPPORT", py_int(96));
    d.insert_str("EPROTO", py_int(71));
    d.insert_str("EPROTONOSUPPORT", py_int(93));
    d.insert_str("EREMCHG", py_int(78));
    d.insert_str("EREMOTE", py_int(66));
    d.insert_str("EREMOTEIO", py_int(121));
    d.insert_str("ERESTART", py_int(85));
    d.insert_str("ERFKILL", py_int(132));
    d.insert_str("ESHUTDOWN", py_int(108));
    d.insert_str("ESOCKTNOSUPPORT", py_int(94));
    d.insert_str("ESRMNT", py_int(69));
    d.insert_str("ESTALE", py_int(116));
    d.insert_str("ESTRPIPE", py_int(86));
    d.insert_str("ETIME", py_int(62));
    d.insert_str("ETIMEDOUT", py_int(110));
    d.insert_str("ETOOMANYREFS", py_int(109));
    d.insert_str("EUCLEAN", py_int(117));
    d.insert_str("EUNATCH", py_int(49));
    d.insert_str("EUSERS", py_int(87));
    d.insert_str("EXFULL", py_int(54));
    d.insert_str("EWOULDBLOCK", py_int(11));
    d.insert_str("EINPROGRESS", py_int(115));
    d.insert_str("EPROTOTYPE", py_int(91));
    d.insert_str("ENOPROTOOPT", py_int(92));
    d.insert_str("EISCONN", py_int(113));
    d.insert_str("__name__", py_str("errno"));
    // `errno.errorcode` — real CPython's reverse mapping (errno NUMBER ->
    // its symbolic NAME string, e.g. `errorcode[2] == 'ENOENT'`). Was
    // missing entirely (`AttributeError`) — `test_errno.py` checks that
    // every constant defined above round-trips through it. Built directly
    // from the constants already inserted, so it can never drift out of
    // sync with them.
    {
        let mut errorcode = PyDict::new();
        for (name, val) in d.iter() {
            if name == "__name__" {
                continue;
            }
            if let PyObject::Int(_) = &*val.borrow() {
                let _ = errorcode.set(val.clone(), py_str(name));
            }
        }
        d.insert_str(
            "errorcode",
            PyObjectRef::new(PyObject::Dict(Box::new(errorcode))),
        );
    }
    d
}
