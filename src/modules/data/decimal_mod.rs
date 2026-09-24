use crate::modules::data::decimal::*;
use crate::modules::data::decimal_ctx::*;
use crate::modules::data::decimal_types::*;
use crate::object::*;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::collections::HashMap;

/// `decimal` module contents. The heavy lifting lives in `decimal.rs`
/// (values/arithmetic), `decimal_types.rs` (the `Decimal` type) and
/// `decimal_ctx.rs` (the `Context` type, signals, flags/traps and the
/// current-context thread-local); this file is the wiring.
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
    d.insert_str("DefaultContext", default_context());
    d.insert_str("BasicContext", basic_context());
    d.insert_str("ExtendedContext", extended_context());
    d.insert_str("DecimalException", decimal_exception_type());
    // Each signal is a real exception class that doubles as the key type
    // for `ctx.flags[...]` / `ctx.traps[...]` (CPython exposes one object
    // for both roles).
    for name in SIGNALS {
        d.insert_str(name, signal_object(name));
    }
    // CPython 3.14 exposes `IEEEContext` as a FACTORY, not a context object.
    d.insert_str(
        "IEEEContext",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "IEEEContext".to_string(),
            func: |_args: &[PyObjectRef]| Ok(ieee_context()),
        }),
    );
    dec_func!("getcontext", |_args| Ok(current_context()));
    dec_func!("setcontext", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("setcontext() missing context argument"));
        }
        if !matches!(&*args[0].borrow(), PyObject::Instance { .. }) {
            return Err(PyError::type_error(
                "setcontext() argument must be a context",
            ));
        }
        set_current_context(&args[0]);
        Ok(py_none())
    });
    // `localcontext(ctx=None, **kwargs)` — a context manager whose
    // `__enter__` installs a copy of the new context and whose `__exit__`
    // restores the previous one, so `with localcontext() as ctx: ctx.prec = 3`
    // affects only the block.
    dec_func!("localcontext", |args| {
        let mut kw: HashMap<String, PyObjectRef> = HashMap::new();
        for arg in args {
            if let PyObject::Dict(d) = &*arg.borrow() {
                for (k, v) in d.iter() {
                    kw.insert(k.str(), v.clone());
                }
            }
        }
        let base = match args
            .iter()
            .find(|a| matches!(&*a.borrow(), PyObject::Instance { .. }))
        {
            Some(ctx) => ctx.clone(),
            None => current_context(),
        };
        // `copy` is fetched as a plain function, so `base` must be passed
        // explicitly (calling it with no arguments raised
        // "TypeError: copy() missing context").
        let copy = base
            .borrow()
            .get_attribute("copy")
            .map_err(|_| PyError::runtime_error("no copy"))?;
        let new_ctx = crate::object::builtin_call(&copy, &[base])?;
        if !kw.is_empty() {
            let mut dict = PyDict::new();
            for (k, v) in kw.iter() {
                let _ = dict.set(py_str(k), v.clone());
            }
            let init = new_ctx
                .borrow()
                .get_attribute("__init__")
                .map_err(|_| PyError::runtime_error("no context initializer"))?;
            crate::object::builtin_call(
                &init,
                &[PyObjectRef::new(PyObject::Dict(Box::new(dict)))],
            )?;
        }
        Ok(make_context_manager(new_ctx))
    });
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
    // Real CPython 3.14 limits (the C `_decimal` values, not the old
    // all-9s placeholders).
    d.insert_str("MAX_PREC", py_int(999999999999999999i64));
    d.insert_str("MAX_EMAX", py_int(425000000i64));
    d.insert_str("MIN_EMIN", py_int(-425000000i64));
    d.insert_str("MIN_ETINY", py_int(-849999999999999999i64));
    d.insert_str("IEEE_CONTEXT_MAX_BITS", py_int(34));
    d.insert_str("HAVE_CONTEXTVAR", py_bool(true));
    d.insert_str("HAVE_THREADS", py_bool(true));
    // `DecimalTuple` — a named tuple type for `Decimal.as_tuple()`.
    d.insert_str("DecimalTuple", decimal_tuple_type());
    d
}
