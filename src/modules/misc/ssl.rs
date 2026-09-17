use crate::object::*;
use std::collections::HashMap;

pub fn create_ssl_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ssl_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // Version constants
    d.insert_str("OPENSSL_VERSION", py_str("OpenSSL 3.0.13 30 Jan 2024"));
    // Real CPython's `ssl.OPENSSL_VERSION_INFO` is a `tuple`, not a `list` —
    // `test_ssl.py` compares it directly against tuple literals at module
    // level (`ssl.OPENSSL_VERSION_INFO >= (3, 0, 0)`), which raises
    // `TypeError: '>=' not supported between instances of 'list' and
    // 'tuple'` when it's a list (lists and tuples don't support ordering
    // comparisons against each other in Python, only same-type).
    d.insert_str(
        "OPENSSL_VERSION_INFO",
        py_tuple(vec![py_int(3), py_int(0), py_int(13), py_int(0), py_int(0)]),
    );
    d.insert_str("OPENSSL_VERSION_NUMBER", py_int(0x300000f0));

    // Feature flags
    d.insert_str("HAS_SNI", py_bool(true));
    d.insert_str("HAS_ALPN", py_bool(true));
    d.insert_str("HAS_TLSv1_3", py_bool(true));
    d.insert_str("HAS_SSLv2", py_bool(false));
    d.insert_str("HAS_SSLv3", py_bool(false));
    d.insert_str("HAS_ECDH", py_bool(true));
    d.insert_str("HAS_NPN", py_bool(false));

    // Certificate verification constants
    d.insert_str("CERT_NONE", py_int(0));
    d.insert_str("CERT_OPTIONAL", py_int(1));
    d.insert_str("CERT_REQUIRED", py_int(2));

    // Protocol constants
    d.insert_str("PROTOCOL_TLS", py_int(2));
    d.insert_str("PROTOCOL_TLS_CLIENT", py_int(5));
    d.insert_str("PROTOCOL_TLS_SERVER", py_int(4));
    d.insert_str("PROTOCOL_SSLv23", py_int(2));
    d.insert_str("PROTOCOL_SSLv3", py_int(3));

    // `ssl._PROTOCOL_NAMES` — real CPython builds this as `{value: name for
    // name, value in _SSLMethod.__members__.items()}` (Lib/ssl.py); used by
    // `test_ssl.py` at module level (`PROTOCOLS = sorted(ssl._PROTOCOL_NAMES)`)
    // to enumerate every known protocol code. Was missing entirely, crashing
    // that file's import outright. Aliased values (PROTOCOL_SSLv23 shares
    // PROTOCOL_TLS's `2`) keep whichever name comes later, same as the real
    // dict-comprehension's last-write-wins behavior.
    {
        let mut proto_names = py_dict();
        if let PyObject::Dict(pd) = &mut *proto_names.borrow_mut() {
            for (value, name) in [
                (3, "PROTOCOL_SSLv3"),
                (2, "PROTOCOL_TLS"),
                (2, "PROTOCOL_SSLv23"),
                (4, "PROTOCOL_TLS_SERVER"),
                (5, "PROTOCOL_TLS_CLIENT"),
            ] {
                let _ = pd.set(py_int(value), py_str(name));
            }
        }
        d.insert_str("_PROTOCOL_NAMES", proto_names);
    }

    // SSL options
    d.insert_str("OP_ALL", py_int(0x80000));
    d.insert_str("OP_NO_SSLv2", py_int(0x100));
    d.insert_str("OP_NO_SSLv3", py_int(0x200));
    d.insert_str("OP_NO_TLSv1", py_int(0x400));
    d.insert_str("OP_NO_TLSv1_1", py_int(0x800));
    d.insert_str("OP_NO_TLSv1_2", py_int(0x1000));
    d.insert_str("OP_NO_TLSv1_3", py_int(0x2000));
    d.insert_str("OP_SINGLE_DH_USE", py_int(0x100000));
    d.insert_str("OP_SINGLE_ECDH_USE", py_int(0x80000));
    d.insert_str("OP_CIPHER_SERVER_PREFERENCE", py_int(0x400000));
    d.insert_str("OP_NO_COMPRESSION", py_int(0x20000));

    // Alert description constants
    d.insert_str("ALERT_DESCRIPTION_CLOSE_NOTIFY", py_int(0));
    d.insert_str("ALERT_DESCRIPTION_HANDSHAKE_FAILURE", py_int(40));
    d.insert_str("ALERT_DESCRIPTION_BAD_CERTIFICATE", py_int(42));
    d.insert_str("ALERT_DESCRIPTION_UNSUPPORTED_CERTIFICATE", py_int(43));
    d.insert_str("ALERT_DESCRIPTION_CERTIFICATE_REVOKED", py_int(44));
    d.insert_str("ALERT_DESCRIPTION_CERTIFICATE_EXPIRED", py_int(45));
    d.insert_str("ALERT_DESCRIPTION_CERTIFICATE_UNKNOWN", py_int(46));
    d.insert_str("ALERT_DESCRIPTION_INTERNAL_ERROR", py_int(80));

    // Verify flags
    d.insert_str("VERIFY_DEFAULT", py_int(0));
    d.insert_str("VERIFY_CRL_CHECK_LEAF", py_int(0x10));
    d.insert_str("VERIFY_CRL_CHECK_CHAIN", py_int(0x20));
    d.insert_str("VERIFY_X509_STRICT", py_int(0x20));

    // Error constants
    d.insert_str("SSL_ERROR_ZERO_RETURN", py_int(0));
    d.insert_str("SSL_ERROR_WANT_READ", py_int(1));
    d.insert_str("SSL_ERROR_WANT_WRITE", py_int(2));
    d.insert_str("SSL_ERROR_WANT_X509_LOOKUP", py_int(3));
    d.insert_str("SSL_ERROR_SYSCALL", py_int(5));
    d.insert_str("SSL_ERROR_SSL", py_int(6));
    d.insert_str("SSL_ERROR_WANT_CONNECT", py_int(7));
    d.insert_str("SSL_ERROR_EOF", py_int(8));
    d.insert_str("SSL_ERROR_INVALID_ERROR_CODE", py_int(20));

    // wrap_socket function — returns the socket as-is
    ssl_func!("wrap_socket", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "wrap_socket() missing required argument: sock",
            ));
        }
        Ok(args[0].clone())
    });

    // get_default_verify_paths — stub
    ssl_func!("get_default_verify_paths", |_| {
        let mut p = HashMap::new();
        p.insert_str(
            "openssl_cafile",
            py_str("/etc/ssl/certs/ca-certificates.crt"),
        );
        p.insert_str("openssl_capath", py_str("/etc/ssl/certs"));
        p.insert_str("ssl_default_verify_paths", py_str("(stub)"));
        Ok(create_module("_VerifyPaths", p))
    });

    // SSLContext stub — returns a module-like object with wrap_socket and other methods
    d.insert_str(
        "SSLContext",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "SSLContext".to_string(),
            func: |_args| {
                let mut ctx_dict = HashMap::new();

                ctx_dict.insert_str(
                    "wrap_socket",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "wrap_socket".to_string(),
                        func: |wargs| {
                            if wargs.is_empty() {
                                return Err(PyError::type_error(
                                    "wrap_socket() missing required argument: sock",
                                ));
                            }
                            Ok(wargs[0].clone())
                        },
                    }),
                );

                ctx_dict.insert_str(
                    "load_default_certs",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "load_default_certs".to_string(),
                        func: |_| Ok(py_none()),
                    }),
                );

                ctx_dict.insert_str(
                    "load_verify_locations",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "load_verify_locations".to_string(),
                        func: |_| Ok(py_none()),
                    }),
                );

                ctx_dict.insert_str(
                    "load_cert_chain",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "load_cert_chain".to_string(),
                        func: |_| Ok(py_none()),
                    }),
                );

                ctx_dict.insert_str(
                    "set_alpn_protocols",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "set_alpn_protocols".to_string(),
                        func: |_| Ok(py_none()),
                    }),
                );

                ctx_dict.insert_str(
                    "set_npn_protocols",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "set_npn_protocols".to_string(),
                        func: |_| Ok(py_none()),
                    }),
                );

                ctx_dict.insert_str(
                    "set_ciphers",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "set_ciphers".to_string(),
                        func: |_| Ok(py_none()),
                    }),
                );

                ctx_dict.insert_str(
                    "set_servername_callback",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "set_servername_callback".to_string(),
                        func: |_| Ok(py_none()),
                    }),
                );

                ctx_dict.insert_str(
                    "get_ca_certs",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "get_ca_certs".to_string(),
                        func: |_| Ok(py_list(vec![])),
                    }),
                );

                ctx_dict.insert_str(
                    "cert_store_stats",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "cert_store_stats".to_string(),
                        func: |_| {
                            let mut s = HashMap::new();
                            s.insert_str("x509_ca", py_int(0));
                            s.insert_str("crl", py_int(0));
                            s.insert_str("x509", py_int(0));
                            Ok(create_module("_CertStoreStats", s))
                        },
                    }),
                );

                ctx_dict.insert_str("check_hostname", py_bool(false));
                ctx_dict.insert_str("verify_mode", py_int(0));

                Ok(create_module("SSLContext", ctx_dict))
            },
        }),
    );

    // SSLSession stub (used by urllib3)
    ssl_func!("SSLSession", |_| Ok(py_none()));

    // CertificateError exception
    d.insert_str(
        "CertificateError",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "CertificateError".to_string(),
            func: |args| {
                Ok(PyObjectRef::new(PyObject::Exception {
                    typ: "CertificateError".to_string(),
                    args: args.to_vec(),
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                }))
            },
        }),
    );

    // SSLError exception
    d.insert_str(
        "SSLError",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "SSLError".to_string(),
            func: |args| {
                Ok(PyObjectRef::new(PyObject::Exception {
                    typ: "SSLError".to_string(),
                    args: args.to_vec(),
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                }))
            },
        }),
    );

    ssl_func!("SSLWantReadError", |args| {
        Ok(PyObjectRef::new(PyObject::Exception {
            typ: "SSLWantReadError".to_string(),
            args: args.to_vec(),
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }))
    });

    ssl_func!("SSLWantWriteError", |args| {
        Ok(PyObjectRef::new(PyObject::Exception {
            typ: "SSLWantWriteError".to_string(),
            args: args.to_vec(),
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }))
    });

    ssl_func!("SSLSyscallError", |args| {
        Ok(PyObjectRef::new(PyObject::Exception {
            typ: "SSLSyscallError".to_string(),
            args: args.to_vec(),
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }))
    });

    ssl_func!("SSLEOFError", |args| {
        Ok(PyObjectRef::new(PyObject::Exception {
            typ: "SSLEOFError".to_string(),
            args: args.to_vec(),
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }))
    });

    // TLSVersion enum stub (int members; real verifies via _ssl).
    {
        let mut tv_dict = HashMap::new();
        for (name, val) in [
            ("MINIMUM_SUPPORTED", -2),
            ("SSLv3", 768),
            ("TLSv1", 769),
            ("TLSv1_1", 770),
            ("TLSv1_2", 771),
            ("TLSv1_3", 772),
            ("MAXIMUM_SUPPORTED", -1),
        ] {
            tv_dict.insert(name.to_string(), py_int(val));
        }
        d.insert_str(
            "TLSVersion",
            PyObjectRef::new(PyObject::Type {
                name: "TLSVersion".to_string(),
                dict: Box::new(str_map_to_typedict(tv_dict)),
                bases: vec![],
                mro: vec![],
            }),
        );
    }
    // _TLSContentType/_TLSAlertType/_TLSMessageType enum stubs (RFC 8446
    // §B.1/B.2/B.3 record-layer/alert/handshake message type codes) — real
    // CPython defines these directly in `Lib/ssl.py` via `@_simple_enum
    // (_IntEnum)`, used by `test_ssl.py`'s `SSLObject.read`
    // msg_callback machinery. Same stub-enum shape as `TLSVersion`/
    // `Purpose` just above (plain int class attributes, not real
    // `IntEnum` instances) — this project has no msg_callback/real-TLS
    // introspection to back richer values with, so only the names/values
    // themselves (needed for `from ssl import _TLSContentType, ...` and
    // any `_TLSContentType.HANDSHAKE`-style comparisons against plain
    // ints) are provided.
    macro_rules! stub_int_enum {
        ($name:expr, [$(($member:expr, $val:expr)),* $(,)?]) => {{
            let mut m = HashMap::new();
            $(m.insert($member.to_string(), py_int($val));)*
            d.insert_str(
                $name,
                PyObjectRef::new(PyObject::Type {
                    name: $name.to_string(),
                    dict: Box::new(str_map_to_typedict(m)),
                    bases: vec![],
                    mro: vec![],
                }),
            );
        }};
    }
    stub_int_enum!(
        "_TLSContentType",
        [
            ("CHANGE_CIPHER_SPEC", 20),
            ("ALERT", 21),
            ("HANDSHAKE", 22),
            ("APPLICATION_DATA", 23),
            ("HEADER", 0x100),
            ("INNER_CONTENT_TYPE", 0x101),
        ]
    );
    stub_int_enum!(
        "_TLSAlertType",
        [
            ("CLOSE_NOTIFY", 0),
            ("UNEXPECTED_MESSAGE", 10),
            ("BAD_RECORD_MAC", 20),
            ("DECRYPTION_FAILED", 21),
            ("RECORD_OVERFLOW", 22),
            ("DECOMPRESSION_FAILURE", 30),
            ("HANDSHAKE_FAILURE", 40),
            ("NO_CERTIFICATE", 41),
            ("BAD_CERTIFICATE", 42),
            ("UNSUPPORTED_CERTIFICATE", 43),
            ("CERTIFICATE_REVOKED", 44),
            ("CERTIFICATE_EXPIRED", 45),
            ("CERTIFICATE_UNKNOWN", 46),
            ("ILLEGAL_PARAMETER", 47),
            ("UNKNOWN_CA", 48),
            ("ACCESS_DENIED", 49),
            ("DECODE_ERROR", 50),
            ("DECRYPT_ERROR", 51),
            ("EXPORT_RESTRICTION", 60),
            ("PROTOCOL_VERSION", 70),
            ("INSUFFICIENT_SECURITY", 71),
            ("INTERNAL_ERROR", 80),
            ("INAPPROPRIATE_FALLBACK", 86),
            ("USER_CANCELED", 90),
            ("NO_RENEGOTIATION", 100),
            ("MISSING_EXTENSION", 109),
            ("UNSUPPORTED_EXTENSION", 110),
            ("CERTIFICATE_UNOBTAINABLE", 111),
            ("UNRECOGNIZED_NAME", 112),
            ("BAD_CERTIFICATE_STATUS_RESPONSE", 113),
            ("BAD_CERTIFICATE_HASH_VALUE", 114),
            ("UNKNOWN_PSK_IDENTITY", 115),
            ("CERTIFICATE_REQUIRED", 116),
            ("NO_APPLICATION_PROTOCOL", 120),
        ]
    );
    stub_int_enum!(
        "_TLSMessageType",
        [
            ("HELLO_REQUEST", 0),
            ("CLIENT_HELLO", 1),
            ("SERVER_HELLO", 2),
            ("HELLO_VERIFY_REQUEST", 3),
            ("NEWSESSION_TICKET", 4),
            ("END_OF_EARLY_DATA", 5),
            ("HELLO_RETRY_REQUEST", 6),
            ("ENCRYPTED_EXTENSIONS", 8),
            ("CERTIFICATE", 11),
            ("SERVER_KEY_EXCHANGE", 12),
            ("CERTIFICATE_REQUEST", 13),
            ("SERVER_DONE", 14),
            ("CERTIFICATE_VERIFY", 15),
            ("CLIENT_KEY_EXCHANGE", 16),
            ("FINISHED", 20),
            ("CERTIFICATE_URL", 21),
            ("CERTIFICATE_STATUS", 22),
            ("SUPPLEMENTAL_DATA", 23),
        ]
    );

    // `CHANNEL_BINDING_TYPES` — real CPython's `Lib/ssl.py` module-level
    // list of supported TLS channel-binding types (used by
    // `SSLSocket.get_channel_binding(cb_type)` to validate its argument).
    // This project has no real channel-binding implementation to back it,
    // but the plain list constant itself (checked via `cb_type not in
    // CHANNEL_BINDING_TYPES`, real trigger: `test_ssl.py`) is enough to
    // stop an `AttributeError` at import/attribute-access time.
    d.insert_str("CHANNEL_BINDING_TYPES", py_list(vec![py_str("tls-unique")]));

    // Additional `HAS_*` capability flags beyond the set already above —
    // same honest-best-effort-for-a-modern-OpenSSL-build rationale (see
    // the "Feature flags" block earlier in this function). Real trigger:
    // `test_ssl.py` references all of these directly on the `ssl` module.
    for (name, val) in [
        ("HAS_TLSv1", true),
        ("HAS_TLSv1_1", true),
        ("HAS_TLSv1_2", true),
        ("HAS_PSK", true),
        ("HAS_PHA", true),
        ("HAS_KEYLOG", true),
        ("HAS_NEVER_CHECK_COMMON_NAME", true),
    ] {
        d.insert_str(name, py_bool(val));
    }

    d.insert_str("__name__", py_str("ssl"));
    d.insert_str(
        "__doc__",
        py_str("TLS/SSL wrapper for socket objects (stub)"),
    );

    // Purpose enum stub (real values wrap _ssl _ASN1Object; sentinels here
    // let `from ssl import Purpose` + SSLContext(Purpose.SERVER_AUTH)
    // import/run; real TLS verification is not implemented).
    {
        let mut purpose_dict = HashMap::new();
        for name in ["SERVER_AUTH", "CLIENT_AUTH"] {
            purpose_dict.insert(
                name.to_string(),
                PyObjectRef::new(PyObject::Instance {
                    typ: PyObjectRef::new(PyObject::Type {
                        name: "Purpose".to_string(),
                        dict: Box::new(str_map_to_typedict(HashMap::new())),
                        bases: vec![],
                        mro: vec![],
                    }),
                    dict: {
                        let mut inner = AttrMap::new();
                        inner.insert_str("name", py_str(name));
                        inner
                    },
                }),
            );
        }
        d.insert_str(
            "Purpose",
            PyObjectRef::new(PyObject::Type {
                name: "Purpose".to_string(),
                dict: Box::new(str_map_to_typedict(purpose_dict)),
                bases: vec![],
                mro: vec![],
            }),
        );
    }

    d
}
