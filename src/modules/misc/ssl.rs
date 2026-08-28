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
    d.insert_str(
        "OPENSSL_VERSION_INFO",
        py_list(vec![py_int(3), py_int(0), py_int(13), py_int(0), py_int(0)]),
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

    d.insert_str("__name__", py_str("ssl"));
    d.insert_str(
        "__doc__",
        py_str("TLS/SSL wrapper for socket objects (stub)"),
    );

    d
}
