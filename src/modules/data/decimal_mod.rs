use crate::object::*;
use std::collections::HashMap;
use num_bigint::{BigInt, Sign};
use num_traits::{Zero, One, Signed, ToPrimitive};
use crate::modules::data::decimal::*;
use crate::modules::data::decimal_types::*;
pub fn create_decimal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dec_func {
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
    d.insert_str("Decimal", get_decimal_type());
    d.insert_str("Context", get_context_type());
    // CPython's decimal module exposes three pre-built contexts as module
    // attributes: DefaultContext (prec 28), BasicContext and ExtendedContext
    // (both prec 9 but with different trap/flag settings). Tests access
    // them via `decimal.BasicContext` etc. for `localcontext` usage. Our
    // earlier stub lacked them entirely (AttributeError). Create them now;
    // BasicContext traps InvalidOperation (so mismatched infs raise), while
    // Extended/Default do not (they return NaN) – the trap flag is consulted
    // by `decimal_add` for the (+Inf)+(-Inf) case.
    {
        let default_ctx = make_context_instance(28, "ROUND_HALF_EVEN");
        let basic_ctx = make_context_instance(9, "ROUND_HALF_UP");
        let extended_ctx = make_context_instance(9, "ROUND_HALF_EVEN");
        // Mark BasicContext as trapping InvalidOperation for test
        // `test_decimal_basiccontext_mismatched_infs_to_nan`.
        if let PyObject::Instance { dict, .. } = &mut *basic_ctx.borrow_mut() {
            let mut traps = crate::object::PyDict::new();
            let _ = traps.set(py_str("InvalidOperation"), py_int(1));
            dict.insert_str("traps", PyObjectRef::new(PyObject::Dict(Box::new(traps))));
            dict.insert_str("_is_basic", py_bool(true));
        }
        if let PyObject::Instance { dict, .. } = &mut *extended_ctx.borrow_mut() {
            dict.insert_str("_is_basic", py_bool(false));
        }
        if let PyObject::Instance { dict, .. } = &mut *default_ctx.borrow_mut() {
            dict.insert_str("_is_basic", py_bool(false));
        }
        d.insert_str("DefaultContext", default_ctx);
        d.insert_str("BasicContext", basic_ctx);
        d.insert_str("ExtendedContext", extended_ctx);
    }
    dec_func!("getcontext", |_args| {
        let (precision, rounding) = current_decimal_context();
        let is_basic = current_is_basic();
        let ctx = make_context_instance(precision, &rounding);
        if let PyObject::Instance { dict, .. } = &mut *ctx.borrow_mut() {
            dict.insert_str("_is_basic", py_bool(is_basic));
        }
        Ok(ctx)
    });
    dec_func!("setcontext", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("setcontext() missing context argument"));
        }
        if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
            let precision = dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
            let rounding = dict
                .get_str("rounding")
                .map(|v| v.str())
                .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
            let is_basic = dict.get_str("_is_basic").map(|v| v.truthy()).unwrap_or(false);
            DECIMAL_CURRENT_CONTEXT.with(|c| {
                *c.borrow_mut() = (precision, rounding);
            });
            DECIMAL_IS_BASIC.with(|c| {
                *c.borrow_mut() = is_basic;
            });
        }
        Ok(py_none())
    });
    // localcontext(ctx=None) — a minimal context-manager-like object; full
    // save/restore-on-exit semantics aren't implemented, only prec/rounding
    // application, which covers the common `with localcontext() as ctx:
    // ctx.prec = N` pattern used for one-off precision changes.
    dec_func!("localcontext", |args| {
        let (precision, rounding, is_basic) = if !args.is_empty() {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                (
                    dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize,
                    dict.get_str("rounding")
                        .map(|v| v.str())
                        .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string()),
                    dict.get_str("_is_basic").map(|v| v.truthy()).unwrap_or(false),
                )
            } else {
                let (p, r) = current_decimal_context();
                (p, r, current_is_basic())
            }
        } else {
            let (p, r) = current_decimal_context();
            (p, r, current_is_basic())
        };
        let ctx = make_context_instance(precision, &rounding);
        if let PyObject::Instance { dict, .. } = &mut *ctx.borrow_mut() {
            dict.insert_str("_is_basic", py_bool(is_basic));
        }
        let mut cm_dict = HashMap::new();
        cm_dict.insert_str(
            "__enter__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__enter__".to_string(),
                func: |args| {
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        let precision =
                            dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
                        let rounding = dict
                            .get_str("rounding")
                            .map(|v| v.str())
                            .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
                        let is_basic = dict.get_str("_is_basic").map(|v| v.truthy()).unwrap_or(false);
                        DECIMAL_CURRENT_CONTEXT.with(|c| {
                            *c.borrow_mut() = (precision, rounding);
                        });
                        DECIMAL_IS_BASIC.with(|c| {
                            *c.borrow_mut() = is_basic;
                        });
                    }
                    Ok(args[0].clone())
                },
            }),
        );
        cm_dict.insert_str(
            "__exit__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__exit__".to_string(),
                func: |_args| {
                    DECIMAL_CURRENT_CONTEXT.with(|c| {
                        *c.borrow_mut() = (28, "ROUND_HALF_EVEN".to_string());
                    });
                    DECIMAL_IS_BASIC.with(|c| {
                        *c.borrow_mut() = false;
                    });
                    Ok(py_bool(false))
                },
            }),
        );
        let cm_typ = PyObjectRef::new(PyObject::Type {
            name: "_ContextManager".to_string(),
            dict: Box::new(str_map_to_typedict(cm_dict)),
            bases: vec![],
            mro: vec![],
        });
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str("prec", py_int(precision as i64));
        inst_dict.insert_str("rounding", py_str(&rounding));
        inst_dict.insert_str("_is_basic", py_bool(is_basic));
        let _ = ctx;
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: cm_typ,
            dict: inst_dict,
        }))
    });
    // Exception types
    d.insert_str(
        "DecimalException",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "DecimalException".to_string(),
            func: crate::object::builtin_make_exception_decimalexception,
        }),
    );
    d.insert_str(
        "InvalidOperation",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "InvalidOperation".to_string(),
            func: crate::object::builtin_make_exception_invalidoperation,
        }),
    );
    d.insert_str(
        "DivisionByZero",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "DivisionByZero".to_string(),
            func: crate::object::builtin_make_exception_decimaldivisionbyzero,
        }),
    );
    d.insert_str(
        "Inexact",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Inexact".to_string(),
            func: crate::object::builtin_make_exception_inexact,
        }),
    );
    d.insert_str(
        "Rounded",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Rounded".to_string(),
            func: crate::object::builtin_make_exception_rounded,
        }),
    );
    d.insert_str(
        "Clamped",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Clamped".to_string(),
            func: crate::object::builtin_make_exception_clamped,
        }),
    );
    d.insert_str(
        "Overflow",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Overflow".to_string(),
            func: crate::object::builtin_make_exception_decimaloverflow,
        }),
    );
    d.insert_str(
        "Underflow",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Underflow".to_string(),
            func: crate::object::builtin_make_exception_decimalunderflow,
        }),
    );
    d.insert_str(
        "FloatOperation",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "FloatOperation".to_string(),
            func: crate::object::builtin_make_exception_floatoperation,
        }),
    );
    // Rounding mode constants — their real string values (that's what
    // CPython's decimal.ROUND_* constants actually are), so equality checks
    // and passing them to quantize()-style calls behave as real code expects.
    for name in [
        "ROUND_CEILING",
        "ROUND_DOWN",
        "ROUND_FLOOR",
        "ROUND_HALF_DOWN",
        "ROUND_HALF_EVEN",
        "ROUND_HALF_UP",
        "ROUND_UP",
        "ROUND_05UP",
    ] {
        d.insert(name.to_string(), py_str(name));
    }
    d.insert_str("MAX_PREC", py_int(999999999999999999i64));
    d.insert_str("MAX_EMAX", py_int(999999999999999999i64));
    d.insert_str("MIN_EMIN", py_int(-999999999999999999i64));
    d
}
