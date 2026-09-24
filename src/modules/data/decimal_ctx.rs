//! Full `decimal.Context` implementation: signals, flags/traps, the
//! context type's method surface, and the numeric helpers those methods
//! share with `Decimal`.
//!
//! The current context is a REAL `Context` instance held in a thread-local
//! (so `getcontext().prec = 5` mutates the context every subsequent
//! operation uses, exactly like CPython, where the current context is a
//! thread-local `Context` object). All context state lives in the
//! instance's own dict: `prec`, `rounding`, `Emin`, `Emax`, `Etiny`,
//! `Etop`, `capitals`, `clamp`, `_is_basic`, plus a `flags` and a `traps`
//! object. Flags/traps are dedicated `ContextFlags`/`ContextTraps`
//! instances rather than plain dicts because CPython code indexes them by
//! SIGNAL OBJECT (`ctx.flags[InvalidOperation]`) as often as by name
//! (`ctx.traps['InvalidOperation']`), and expects `.clear()`, `len()`,
//! iteration and `in` on them.

use crate::modules::data::decimal::*;
use crate::modules::data::decimal_types::get_context_type;
use crate::object::*;
use num_bigint::BigInt;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::collections::HashMap;

/// The twelve decimal signals, in CPython's own order.
pub const SIGNALS: [&str; 12] = [
    "Clamped",
    "DivisionByZero",
    "DivisionImpossible",
    "DivisionUndefined",
    "Inexact",
    "InvalidOperation",
    "Overflow",
    "Rounded",
    "Subnormal",
    "Underflow",
    "FloatOperation",
    "ConversionSyntax",
];

thread_local! {
    /// The signal objects (`decimal.InvalidOperation` etc.), created lazily
    /// and cached so `ctx.flags[InvalidOperation]` and
    /// `ctx.flags[decimal.InvalidOperation]` are the same key.
    pub static DECIMAL_SIGNALS: std::cell::RefCell<HashMap<String, PyObjectRef>> =
        std::cell::RefCell::new(HashMap::new());
    pub static DECIMAL_FLAGS_TYPE: std::cell::RefCell<Option<PyObjectRef>> =
        std::cell::RefCell::new(None);
    pub static DECIMAL_TRAPS_TYPE: std::cell::RefCell<Option<PyObjectRef>> =
        std::cell::RefCell::new(None);
    pub static DECIMAL_EXCEPTION_TYPE: std::cell::RefCell<Option<PyObjectRef>> =
        std::cell::RefCell::new(None);
    /// The live current context object.
    pub static DECIMAL_CURRENT_CTX: std::cell::RefCell<Option<PyObjectRef>> =
        std::cell::RefCell::new(None);
    /// `localcontext()` nesting: saved contexts, innermost last.
    pub static DECIMAL_CTX_STACK: std::cell::RefCell<Vec<PyObjectRef>> =
        std::cell::RefCell::new(Vec::new());
}

/// `decimal.DecimalException` — the base of every decimal signal class.
pub fn decimal_exception_type() -> PyObjectRef {
    if let Some(existing) = DECIMAL_EXCEPTION_TYPE.with(|c| c.borrow().clone()) {
        return existing;
    }
    let mut dict: HashMap<String, PyObjectRef> = HashMap::new();
    dict.insert_str("__module__", py_str("decimal"));
    let bases = crate::modules::get_builtin_class("ArithmeticError")
        .map(|b| vec![b])
        .unwrap_or_default();
    let typ = PyObjectRef::new(PyObject::Type {
        name: "DecimalException".to_string(),
        dict: Box::new(str_map_to_typedict(dict)),
        bases,
        mro: vec![],
    });
    DECIMAL_EXCEPTION_TYPE.with(|c| *c.borrow_mut() = Some(typ.clone()));
    typ
}

/// The `decimal.<Signal>` class for `name` — a real exception class (so
/// `raise decimal.Inexact`, `except decimal.InvalidOperation` and
/// `issubclass(decimal.Overflow, decimal.DecimalException)` all work) that
/// also doubles as the signal object used to index `ctx.flags`/`ctx.traps`
/// (CPython uses one object for both roles). Signals compare by their
/// numeric order, which is what `sorted(ctx.traps)` relies on.
pub fn signal_object(name: &str) -> PyObjectRef {
    if let Some(existing) = DECIMAL_SIGNALS.with(|m| m.borrow().get(name).cloned()) {
        return existing;
    }
    let index = SIGNALS.iter().position(|s| *s == name).unwrap_or(0);
    let mut dict: HashMap<String, PyObjectRef> = HashMap::new();
    dict.insert_str("name", py_str(name));
    dict.insert_str("__module__", py_str("decimal"));
    let signal_index = py_int(index as i64);
    // A capturing closure (the `BuiltinFunction.func` field is a plain fn
    // pointer, so a capture needs `PyObject::Closure`).
    dict.insert_str(
        "__lt__",
        PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
            move |args: &[PyObjectRef]| {
                let other = args
                    .get(1)
                    .and_then(|v| v.borrow().get_attribute("_signal_index").ok())
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1);
                Ok(py_bool((index as i64) < other))
            },
        ))),
    );
    dict.insert_str("__index__", signal_index.clone());
    dict.insert_str("_signal_index", signal_index);
    let bases = vec![decimal_exception_type()];
    let typ = PyObjectRef::new(PyObject::Type {
        name: name.to_string(),
        dict: Box::new(str_map_to_typedict(dict)),
        bases,
        mro: vec![],
    });
    DECIMAL_SIGNALS.with(|m| {
        m.borrow_mut().insert(name.to_string(), typ.clone());
    });
    typ
}

/// The signal name a lookup key denotes: a plain string, or any object
/// exposing a `name` attribute (which is what the signal singletons do).
pub fn signal_name_of(key: &PyObjectRef) -> Option<String> {
    if matches!(&*key.borrow(), PyObject::Str(_)) {
        return Some(key.str());
    }
    if let Some(name) = key.borrow().get_attribute("name").ok().map(|v| v.str()) {
        if SIGNALS.contains(&name.as_str()) {
            return Some(name);
        }
    }
    // The module exposes each signal as its exception constructor
    // (`decimal.Inexact` is a callable that builds the exception), so a
    // builtin function's own name identifies the signal.
    match &*key.borrow() {
        PyObject::BuiltinFunction { name, .. } if SIGNALS.contains(&name.as_str()) => {
            Some(name.clone())
        }
        _ => None,
    }
}

/// Build the flags/traps container type. The methods are plain `fn`s (the
/// `BuiltinFunction.func` field is a non-capturing fn pointer), so they
/// recover "flags" vs "traps" from the container's own `_kind` entry
/// instead of capturing the kind.
fn signal_type() -> PyObjectRef {
    if let Some(existing) = DECIMAL_FLAGS_TYPE.with(|c| c.borrow().clone()) {
        return existing;
    }
    fn container_parts(args: &[PyObjectRef]) -> PyResult<(PyObjectRef, &'static str)> {
        let ctx = args[0]
            .borrow()
            .get_attribute("_ctx")
            .map_err(|_| PyError::runtime_error("no context"))?;
        let kind = args[0]
            .borrow()
            .get_attribute("_kind")
            .map(|v| v.str())
            .map_err(|_| PyError::runtime_error("no signal kind"))?;
        let kind: &'static str = if kind == "flags" { "flags" } else { "traps" };
        Ok((ctx, kind))
    }
    let mut dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! meth {
        ($name:expr, $f:expr) => {
            dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $f,
                }),
            );
        };
    }
    meth!("__getitem__", |args: &[PyObjectRef]| {
        let (ctx, kind) = container_parts(args)?;
        let name = signal_name_of(&args[1]).ok_or_else(|| PyError::key_error(args[1].str()))?;
        read_signal(&ctx, kind, &name)
    });
    meth!("__setitem__", |args: &[PyObjectRef]| {
        let (ctx, kind) = container_parts(args)?;
        let name = signal_name_of(&args[1]).ok_or_else(|| PyError::key_error(args[1].str()))?;
        let value = args.get(2).map(|v| v.truthy()).unwrap_or(true);
        write_signal(&ctx, kind, &name, value);
        Ok(py_none())
    });
    meth!("__contains__", |args: &[PyObjectRef]| {
        Ok(py_bool(signal_name_of(&args[1]).is_some()))
    });
    // Two flags/traps objects compare equal when they hold the same signals
    // (`self.assertEqual(c.flags, d.flags)` after a copy).
    meth!("__eq__", |args: &[PyObjectRef]| {
        if args.len() < 2 {
            return Ok(py_bool(false));
        }
        let same = match (
            container_parts(args),
            args.get(1)
                .map(|other| container_parts(&[other.clone(), py_none()]))
                .unwrap_or(Err(PyError::type_error("missing operand"))),
        ) {
            (Ok((left_ctx, left_kind)), Ok((right_ctx, right_kind))) => {
                let mut equal = true;
                for name in SIGNALS {
                    let l =
                        read_signal(&left_ctx, left_kind, name).unwrap_or_else(|_| py_bool(false));
                    let r = read_signal(&right_ctx, right_kind, name)
                        .unwrap_or_else(|_| py_bool(false));
                    if l.truthy() != r.truthy() {
                        equal = false;
                        break;
                    }
                }
                equal
            }
            _ => false,
        };
        Ok(py_bool(same))
    });
    meth!("__len__", |_args: &[PyObjectRef]| Ok(py_int(
        SIGNALS.len() as i64
    )));
    meth!("__iter__", |_args: &[PyObjectRef]| {
        let items: Vec<PyObjectRef> = SIGNALS.iter().map(|s| signal_object(s)).collect();
        Ok(PyObjectRef::new(PyObject::List(items)))
    });
    meth!("clear", |args: &[PyObjectRef]| {
        let (ctx, kind) = container_parts(args)?;
        for name in SIGNALS {
            write_signal(&ctx, kind, name, false);
        }
        Ok(py_none())
    });
    meth!("copy", |args: &[PyObjectRef]| {
        let (ctx, kind) = container_parts(args)?;
        let mut d = PyDict::new();
        for name in SIGNALS {
            let _ = d.set(py_str(name), read_signal(&ctx, kind, name)?);
        }
        Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
    });
    meth!("keys", |_args: &[PyObjectRef]| {
        let items: Vec<PyObjectRef> = SIGNALS.iter().map(|s| signal_object(s)).collect();
        Ok(PyObjectRef::new(PyObject::List(items)))
    });
    meth!("values", |args: &[PyObjectRef]| {
        let (ctx, kind) = container_parts(args)?;
        let mut items = Vec::new();
        for name in SIGNALS {
            items.push(read_signal(&ctx, kind, name)?);
        }
        Ok(PyObjectRef::new(PyObject::List(items)))
    });
    meth!("items", |args: &[PyObjectRef]| {
        let (ctx, kind) = container_parts(args)?;
        let mut items = Vec::new();
        for name in SIGNALS {
            items.push(PyObjectRef::new(PyObject::Tuple(vec![
                signal_object(name),
                read_signal(&ctx, kind, name)?,
            ])));
        }
        Ok(PyObjectRef::new(PyObject::List(items)))
    });
    let typ = PyObjectRef::new(PyObject::Type {
        name: "ContextFlags".to_string(),
        dict: Box::new(str_map_to_typedict(dict)),
        bases: vec![],
        mro: vec![],
    });
    DECIMAL_FLAGS_TYPE.with(|c| *c.borrow_mut() = Some(typ.clone()));
    DECIMAL_TRAPS_TYPE.with(|c| *c.borrow_mut() = Some(typ.clone()));
    typ
}

fn make_signal_container(ctx: &PyObjectRef, kind: &str) -> PyObjectRef {
    let typ = signal_type();
    let mut dict = AttrMap::new();
    dict.insert_str("_ctx", ctx.clone());
    dict.insert_str("_kind", py_str(kind));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

/// `ctx.flags[signal]` / `ctx.traps[signal]`.
pub fn read_signal(ctx: &PyObjectRef, kind: &str, name: &str) -> PyResult<PyObjectRef> {
    let key = if kind == "flags" { "flags" } else { "traps" };
    let container = ctx
        .borrow()
        .get_attribute(key)
        .map_err(|_| PyError::runtime_error("context has no flags/traps"))?;
    let stored = container
        .borrow()
        .get_attribute(name)
        .map_err(|_| PyError::runtime_error("no signal"))?;
    Ok(stored)
}

/// Set `ctx.flags[signal]` (or a trap) to `value`.
pub fn write_signal(ctx: &PyObjectRef, kind: &str, name: &str, value: bool) {
    let key = if kind == "flags" { "flags" } else { "traps" };
    if let Ok(container) = ctx.borrow().get_attribute(key) {
        if let PyObject::Instance { dict, .. } = &mut *container.borrow_mut() {
            dict.insert_str(name, py_bool(value));
        }
    }
}

/// Is `signal` trapped in `ctx`?
pub fn signal_trapped(ctx: &PyObjectRef, name: &str) -> bool {
    read_signal(ctx, "traps", name)
        .map(|v| v.truthy())
        .unwrap_or(false)
}

/// Raise `signal` if it is trapped in `ctx`; always record the flag.
pub fn check_signal(ctx: &PyObjectRef, name: &str, message: &str) -> PyResult<()> {
    write_signal(ctx, "flags", name, true);
    if signal_trapped(ctx, name) {
        return Err(decimal_signal_error(name, message));
    }
    Ok(())
}

/// The decimal exception object for a signal name.
pub fn decimal_signal_error(name: &str, message: &str) -> PyError {
    let typ = match name {
        "DivisionByZero" => "DivisionByZero",
        "Overflow" => "Overflow",
        "Underflow" => "Underflow",
        "Inexact" => "Inexact",
        "Rounded" => "Rounded",
        "Clamped" => "Clamped",
        "Subnormal" => "Subnormal",
        "FloatOperation" => "FloatOperation",
        "ConversionSyntax" => "ConversionSyntax",
        "DivisionImpossible" => "InvalidOperation",
        "DivisionUndefined" => "InvalidOperation",
        _ => "InvalidOperation",
    };
    PyError::Exception(
        typ.to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: typ.to_string(),
            args: vec![py_str(message)],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}

fn ctx_dict_field(ctx: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    match &*ctx.borrow() {
        PyObject::Instance { dict, .. } => dict.get_str(name).cloned(),
        _ => None,
    }
}

fn ctx_set_field(ctx: &PyObjectRef, name: &str, value: PyObjectRef) {
    if let PyObject::Instance { dict, .. } = &mut *ctx.borrow_mut() {
        dict.insert_str(name, value);
    }
}

pub fn ctx_prec(ctx: &PyObjectRef) -> usize {
    ctx_dict_field(ctx, "prec")
        .and_then(|v| v.as_i64())
        .unwrap_or(28)
        .max(1) as usize
}

pub fn ctx_rounding(ctx: &PyObjectRef) -> String {
    ctx_dict_field(ctx, "rounding")
        .map(|v| v.str())
        .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string())
}

pub fn ctx_int_field(ctx: &PyObjectRef, name: &str, default: i64) -> i64 {
    ctx_dict_field(ctx, name)
        .and_then(|v| v.as_i64())
        .unwrap_or(default)
}

/// `Etiny = Emin - prec + 1`; kept in the dict so `ctx.Etiny` reads back.
pub fn ctx_update_etiny(ctx: &PyObjectRef) {
    let emin = ctx_int_field(ctx, "Emin", -999999);
    let prec = ctx_prec(ctx) as i64;
    let etiny = emin - prec + 1;
    ctx_set_field(ctx, "Etiny", py_int(etiny));
    let emax = ctx_int_field(ctx, "Emax", 999999);
    ctx_set_field(ctx, "Etop", py_int(emax - prec + 1));
}

/// A `Context` with CPython 3.14's field values for the three standard
/// contexts (verified against `_decimal`: default `Emin`/`Emax` are
/// +/-999999, and the default trap set is
/// {InvalidOperation, DivisionByZero, Overflow}).
pub fn new_context(
    prec: i64,
    rounding: &str,
    emin: i64,
    emax: i64,
    capitals: bool,
    clamp: bool,
    traps: &[&str],
) -> PyObjectRef {
    let typ = get_context_type();
    let ctx = PyObjectRef::new(PyObject::Instance {
        typ,
        dict: AttrMap::new(),
    });
    ctx_set_field(&ctx, "prec", py_int(prec));
    ctx_set_field(&ctx, "rounding", py_str(rounding));
    ctx_set_field(&ctx, "Emin", py_int(emin));
    ctx_set_field(&ctx, "Emax", py_int(emax));
    ctx_set_field(&ctx, "capitals", py_int(if capitals { 1 } else { 0 }));
    ctx_set_field(&ctx, "clamp", py_int(if clamp { 1 } else { 0 }));
    ctx_set_field(&ctx, "_is_basic", py_bool(false));
    ctx_update_etiny(&ctx);
    let flags = make_signal_container(&ctx, "flags");
    ctx_set_field(&ctx, "flags", flags);
    let traps_obj = make_signal_container(&ctx, "traps");
    ctx_set_field(&ctx, "traps", traps_obj);
    for name in SIGNALS {
        write_signal(&ctx, "flags", name, false);
        write_signal(&ctx, "traps", name, traps.contains(&name));
    }
    ctx
}

/// The trap set CPython's C `_decimal` uses for `DefaultContext`,
/// `getcontext()` and a fresh `Context()`.
pub const DEFAULT_TRAPS: [&str; 3] = ["InvalidOperation", "DivisionByZero", "Overflow"];

/// CPython's `DefaultContext`.
pub fn default_context() -> PyObjectRef {
    new_context(
        28,
        "ROUND_HALF_EVEN",
        -999999,
        999999,
        true,
        false,
        &DEFAULT_TRAPS,
    )
}

/// CPython's `BasicContext` (traps Clamped and Underflow as well).
pub fn basic_context() -> PyObjectRef {
    new_context(
        9,
        "ROUND_HALF_UP",
        -999999,
        999999,
        true,
        false,
        &[
            "Clamped",
            "InvalidOperation",
            "DivisionByZero",
            "Overflow",
            "Underflow",
        ],
    )
}

/// CPython's `ExtendedContext` (no traps at all).
pub fn extended_context() -> PyObjectRef {
    new_context(9, "ROUND_HALF_EVEN", -999999, 999999, true, false, &[])
}

/// CPython 3.14's `IEEEContext(...)` — a FACTORY, not a context object.
pub fn ieee_context() -> PyObjectRef {
    new_context(
        34,
        "ROUND_HALF_EVEN",
        -6143,
        6144,
        false,
        false,
        &["Inexact", "Rounded", "Subnormal", "Underflow"],
    )
}

/// The live current context, created on first use.
pub fn current_context() -> PyObjectRef {
    if let Some(existing) = DECIMAL_CURRENT_CTX.with(|c| c.borrow().clone()) {
        return existing;
    }
    let ctx = default_context();
    DECIMAL_CURRENT_CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    ctx
}

pub fn set_current_context(ctx: &PyObjectRef) {
    DECIMAL_CURRENT_CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    DECIMAL_CURRENT_CONTEXT.with(|c| {
        *c.borrow_mut() = (ctx_prec(ctx), ctx_rounding(ctx));
    });
    DECIMAL_IS_BASIC.with(|c| {
        *c.borrow_mut() = ctx_dict_field(ctx, "_is_basic")
            .map(|v| v.truthy())
            .unwrap_or(false);
    });
}

/// Round `v` to `prec` digits with `rounding`, then apply the context's
/// exponent limits (subnormal / underflow / overflow), recording signals and
/// honouring traps. This is the single place every arithmetic result passes
/// through, so flags stay consistent.
pub fn apply_context(ctx: &PyObjectRef, v: DecValue) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite {
        return Ok(v);
    }
    let prec = ctx_prec(ctx);
    let rounding = ctx_rounding(ctx);
    let mut out = round_decvalue(&v, prec, &rounding);
    let dropped = digit_count(&v.coeff) > digit_count(&out.coeff);
    if dropped {
        // Digits were discarded: Rounded always, Inexact when the discarded
        // part was non-zero.
        check_signal(ctx, "Rounded", "Rounded")?;
        let lost = !decimal_rounding_was_exact(&v, &out);
        if lost {
            check_signal(ctx, "Inexact", "Inexact")?;
        }
    }
    let etiny = ctx_int_field(ctx, "Etiny", -1000000007);
    let emax = ctx_int_field(ctx, "Emax", 999999);
    let clamp = ctx_dict_field(ctx, "clamp")
        .map(|v| v.truthy())
        .unwrap_or(false);
    if !out.is_zero() {
        let adjusted = out.exp + digit_count(&out.coeff) as i64 - 1;
        if adjusted > emax {
            // Overflow: infinity (or the largest finite value when trapped).
            if signal_trapped(ctx, "Overflow") {
                check_signal(ctx, "Overflow", "Overflow")?;
                if signal_trapped(ctx, "Overflow") {
                    let cap = &BigInt::from(10).pow(prec as u32) - 1;
                    return Ok(DecValue {
                        special: DecSpecial::Finite,
                        sign: out.sign,
                        coeff: cap,
                        exp: emax - prec as i64 + 1,
                    });
                }
            } else {
                write_signal(ctx, "flags", "Overflow", true);
                write_signal(ctx, "flags", "Rounded", true);
                write_signal(ctx, "flags", "Inexact", true);
                return Ok(DecValue::infinity(out.sign));
            }
        }
        if adjusted < etiny {
            // Subnormal: reduce the exponent, possibly to Etiny.
            let target_exp = etiny.min(out.exp);
            if out.exp < target_exp {
                out = shift_exp(&out, target_exp - out.exp);
            }
            write_signal(ctx, "flags", "Subnormal", true);
            write_signal(ctx, "flags", "Inexact", true);
            write_signal(ctx, "flags", "Underflow", true);
            if signal_trapped(ctx, "Underflow") {
                return Err(decimal_signal_error("Underflow", "Underflow"));
            }
            if signal_trapped(ctx, "Inexact") {
                return Err(decimal_signal_error("Inexact", "Inexact"));
            }
        }
    }
    if clamp && ctx_int_field(ctx, "Etop", 0) <= 0 {
        // `clamp` pins the exponent range; our simplified model only needs to
        // avoid an exponent above Etop for subnormal results.
        let etop = ctx_int_field(ctx, "Etop", 0);
        if out.exp > etop && !out.is_zero() {
            out = shift_exp(&out, etop - out.exp);
        }
    }
    Ok(out)
}

/// Did rounding discard a NON-zero remainder (i.e. is the result inexact)?
fn decimal_rounding_was_exact(before: &DecValue, after: &DecValue) -> bool {
    // Recompute at the rounded exponent: exact iff the original digits are
    // recoverable, i.e. `before` had no digits below `after.exp`.
    if before.exp >= after.exp {
        return true;
    }
    let drop = before.exp - after.exp;
    let divisor = ten_pow(drop);
    &before.coeff % &divisor == BigInt::zero()
}

/// Multiply a value by `10^n` for `n > 0` (append zeros) or divide it by
/// `10^-n` when `n < 0`.
pub fn shift_exp(v: &DecValue, n: i64) -> DecValue {
    if v.special != DecSpecial::Finite || n == 0 {
        return v.clone();
    }
    if n > 0 {
        DecValue {
            special: v.special.clone(),
            sign: v.sign,
            coeff: &v.coeff * ten_pow(n),
            exp: v.exp - n,
        }
    } else {
        DecValue {
            special: v.special.clone(),
            sign: v.sign,
            coeff: &v.coeff / ten_pow(-n),
            exp: v.exp - n,
        }
    }
}

/// Number of significant digits currently held (at least 1).
pub fn significant_digits(v: &DecValue) -> usize {
    digit_count(&v.coeff).max(1)
}

/// `adjusted()` — the exponent of the most significant digit.
pub fn decimal_adjusted(v: &DecValue) -> PyResult<i64> {
    match v.special {
        DecSpecial::Infinity => Ok(v.exp),
        DecSpecial::QNaN | DecSpecial::SNaN => Err(decimal_invalid_op("adjusted")),
        DecSpecial::Finite => Ok(if v.is_zero() {
            v.exp
        } else {
            v.exp + digit_count(&v.coeff) as i64 - 1
        }),
    }
}

/// Round a finite value to an integral exponent (0 or `exponent`), the
/// shared body of `to_integral`, `to_integral_value` and `quantize`.
pub fn decimal_to_integral(
    ctx: &PyObjectRef,
    v: &DecValue,
    exponent: i64,
    rounding: &str,
) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite {
        return Ok(v.clone());
    }
    if v.exp >= exponent {
        return Ok(v.clone());
    }
    let out = round_decvalue(v, 0, rounding);
    let mut out = shift_exp(&out, exponent);
    // `round_decvalue(v, 0, ...)` drops every digit; the exponent has to be
    // set explicitly afterwards.
    out.exp = exponent;
    if digit_count(&out.coeff) > ctx_prec(ctx) && out.is_zero() {
        check_signal(ctx, "Inexact", "Inexact")?;
    }
    Ok(out)
}

/// Round `v` so that at most `ndigits` digits remain AFTER the decimal
/// point (CPython's `Decimal.__round__(ndigits)` semantics: the target
/// exponent is `-ndigits`, and `ndigits = None` means "to an integer").
pub fn decimal_round(
    v: &DecValue,
    ndigits: Option<i64>,
    rounding: Option<&str>,
) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite {
        return Ok(v.clone());
    }
    let target_exp = match ndigits {
        None => 0,
        Some(n) => -n,
    };
    let r = rounding.unwrap_or("ROUND_HALF_EVEN");
    Ok(round_to_exponent(v, target_exp, r))
}

/// Round `v` to a fixed EXPONENT (the shared body of `round`,
/// `to_integral_value`, `floor`/`ceil`/`trunc` and `quantize`).
pub fn round_to_exponent(v: &DecValue, exponent: i64, rounding: &str) -> DecValue {
    if v.special != DecSpecial::Finite {
        return v.clone();
    }
    // Even when the value is already integral at a COARSER exponent, CPython
    // re-expresses it at the requested one (`Decimal('1.5').quantize(
    // Decimal('0.01'))` is `1.50`, not `1.5`), so the exponent is always
    // normalized to the target.
    if v.is_zero() {
        let mut out = v.clone();
        out.exp = exponent;
        return out;
    }
    if v.exp > exponent {
        return DecValue {
            special: DecSpecial::Finite,
            sign: v.sign,
            coeff: &v.coeff * ten_pow(v.exp - exponent),
            exp: exponent,
        };
    }
    if v.exp == exponent {
        return v.clone();
    }
    // `value / 10**exponent` is `coeff / 10**(exponent - v.exp)`: a fraction
    // that has to be rounded to an integer with the requested mode (which is
    // exactly what CPython's "round to this exponent" does).
    let divisor = ten_pow(exponent - v.exp);
    let coeff = round_ratio_to_int(&v.coeff, &divisor, rounding, v.sign);
    DecValue {
        special: DecSpecial::Finite,
        sign: v.sign,
        coeff,
        exp: exponent,
    }
}

/// Round the non-negative ratio `num / den` to an integer, ties per
/// `rounding` (half-even by default).
fn round_ratio_to_int(num: &BigInt, den: &BigInt, rounding: &str, negative: bool) -> BigInt {
    let quotient = num / den;
    let remainder = num % den;
    if remainder.is_zero() {
        return quotient;
    }
    let twice: BigInt = &remainder * 2;
    let up = match rounding {
        "ROUND_HALF_UP" => twice >= *den,
        "ROUND_HALF_DOWN" => twice > *den,
        "ROUND_HALF_EVEN" => {
            use std::cmp::Ordering;
            match twice.cmp(den) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => (&quotient % 2) != BigInt::from(0),
            }
        }
        "ROUND_UP" => true,
        "ROUND_DOWN" => false,
        "ROUND_CEILING" => !negative,
        "ROUND_FLOOR" => negative,
        "ROUND_05UP" => {
            let last = &quotient % 10;
            last == BigInt::from(0) || last == BigInt::from(5)
        }
        _ => {
            use std::cmp::Ordering;
            match twice.cmp(den) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => (&quotient % 2) != BigInt::from(0),
            }
        }
    };
    if up {
        quotient + 1
    } else {
        quotient
    }
}

/// `Decimal.fma(a, b, c)` — fused multiply-add: `a*b + c` computed with a
/// SINGLE final rounding.
pub fn decimal_fma(
    ctx: &PyObjectRef,
    a: &DecValue,
    b: &DecValue,
    c: &DecValue,
) -> PyResult<DecValue> {
    if a.special != DecSpecial::Finite
        || b.special != DecSpecial::Finite
        || c.special != DecSpecial::Finite
    {
        if a.special == DecSpecial::SNaN || b.special == DecSpecial::SNaN {
            return Err(decimal_invalid_op("fma"));
        }
        if c.special == DecSpecial::SNaN {
            return Err(decimal_invalid_op("fma"));
        }
        if (a.special == DecSpecial::Infinity && b.is_zero())
            || (b.special == DecSpecial::Infinity && a.is_zero())
        {
            return Err(decimal_invalid_op("fma"));
        }
    }
    if a.special == DecSpecial::Infinity || b.special == DecSpecial::Infinity {
        return Ok(a.clone());
    }
    if c.special == DecSpecial::Infinity {
        return Ok(c.clone());
    }
    if b.is_zero() {
        if c.is_zero() {
            return Ok(DecValue {
                special: DecSpecial::Finite,
                sign: a.sign,
                coeff: BigInt::zero(),
                exp: c.exp.min(a.exp),
            });
        }
        return Ok(c.clone());
    }
    // Exact product, then add, then round ONCE.
    let product = DecValue {
        special: DecSpecial::Finite,
        sign: a.sign != b.sign,
        coeff: &a.coeff * &b.coeff,
        exp: a.exp + b.exp,
    };
    let sum = decimal_add_exact(&product, c)?;
    apply_context(ctx, sum)
}

/// Exact (unrounded) addition of two finite values.
pub fn decimal_add_exact(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.special != DecSpecial::Finite || b.special != DecSpecial::Finite {
        return decimal_add(a, b);
    }
    let (a_scaled, b_scaled, exp) = decval_align(a, b);
    let sum = if a.sign == b.sign {
        a_scaled + b_scaled
    } else {
        a_scaled - b_scaled
    };
    let negative = a.sign != b.sign && sum < BigInt::zero();
    Ok(DecValue {
        special: DecSpecial::Finite,
        sign: negative,
        coeff: sum.abs(),
        exp,
    })
}

/// `Decimal.scaleb(n)` — exponent shift by `n`, rounded to context.
pub fn decimal_scaleb(ctx: &PyObjectRef, v: &DecValue, n: i64) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite {
        return Ok(v.clone());
    }
    let shifted = DecValue {
        exp: v.exp + n,
        ..v.clone()
    };
    apply_context(ctx, shifted)
}

/// `Decimal.shift(other)` — `self * 10**int(other)`, exact.
pub fn decimal_shift(ctx: &PyObjectRef, v: &DecValue, other: &DecValue) -> PyResult<DecValue> {
    if other.special != DecSpecial::Finite || other.coeff.to_u32().is_none() {
        return Err(decimal_invalid_op("shift"));
    }
    let n = other.coeff.to_u32().unwrap() as i64;
    let n = if other.sign { -n } else { n };
    if v.special != DecSpecial::Finite {
        return Ok(v.clone());
    }
    let shifted = DecValue {
        exp: v.exp + n,
        ..v.clone()
    };
    apply_context(ctx, shifted)
}

/// `Decimal.rotate(other)` — rotate the coefficient digits.
pub fn decimal_rotate(v: &DecValue, other: &DecValue) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite || other.special != DecSpecial::Finite {
        return Err(decimal_invalid_op("rotate"));
    }
    let digits = significant_digits(v);
    let mut n = other.coeff.to_i64().unwrap_or(0);
    if other.sign {
        n = -n;
    }
    let len = digits as i64;
    if len == 0 {
        return Ok(v.clone());
    }
    let mut k = n % len;
    if k < 0 {
        k += len;
    }
    let s = format!("{:0width$}", v.coeff, width = digits);
    let rotated = format!("{}{}", &s[k as usize..], &s[..k as usize]);
    let coeff: BigInt = rotated.parse().unwrap_or_else(|_| BigInt::zero());
    Ok(DecValue {
        special: DecSpecial::Finite,
        sign: v.sign,
        coeff,
        exp: v.exp,
    })
}

/// `remainder_near(a, b)` — the remainder nearest to zero among the
/// candidates `a - k*b`, with ties going AWAY from zero. CPython 3.14's
/// signature takes only two arguments (`10.remainder_near(6) == -2`).
pub fn decimal_remainder_near(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.special != DecSpecial::Finite || b.special != DecSpecial::Finite {
        return Err(decimal_invalid_op("remainder_near"));
    }
    if b.is_zero() {
        return Err(decimal_invalid_op("remainder_near"));
    }
    let rem = decimal_remainder(a, b)?;
    // Compare |rem| with |b|/2: when |rem| exceeds half of |b| the nearest
    // multiple lies on the other side, and an exact tie steps away from zero.
    let abs_rem = decimal_copy_abs(&rem);
    let half = decimal_div(&decimal_copy_abs(b), &dec_from_int(2))?;
    use std::cmp::Ordering;
    match decimal_compare(&abs_rem, &half) {
        Some(Ordering::Greater) => {
            // The nearest multiple is on the other side: subtract `b` when
            // that reduces |rem| (same signs), add it otherwise.
            if rem.sign == b.sign {
                decimal_sub(&rem, b)
            } else {
                decimal_add(&rem, b)
            }
        }
        Some(Ordering::Equal) => {
            // Exact tie: step away from zero, i.e. whichever of `rem - b` and
            // `rem + b` has the larger magnitude.
            let down = decimal_sub(&rem, b)?;
            let up = decimal_add(&rem, b)?;
            if decimal_copy_abs(&down).coeff > decimal_copy_abs(&up).coeff {
                Ok(down)
            } else {
                Ok(up)
            }
        }
        _ => Ok(rem),
    }
}

/// A finite Decimal from a small integer.
pub fn dec_from_int(n: i64) -> DecValue {
    DecValue {
        special: DecSpecial::Finite,
        sign: n < 0,
        coeff: BigInt::from(n.abs()),
        exp: 0,
    }
}

/// `Decimal.ln()` / `log10()` / `exp()` — float-precision results, rounded
/// back to the context (documented approximation: these are the only
/// transcendental operations in the module).
fn transcendental(ctx: &PyObjectRef, v: &DecValue, which: &str) -> PyResult<DecValue> {
    if v.special == DecSpecial::SNaN {
        return Err(decimal_invalid_op(which));
    }
    if v.special != DecSpecial::Finite {
        if which == "exp" {
            return Ok(DecValue::nan());
        }
        return Err(decimal_invalid_op(which));
    }
    let sign = if v.sign { -1.0 } else { 1.0 };
    let x = sign * decval_to_f64(v);
    let y = match which {
        "ln" => x.ln(),
        "log10" => x.log10(),
        "log2" => x.log2(),
        "exp" => x.exp(),
        _ => x.sqrt(),
    };
    if y.is_nan() {
        return Ok(DecValue::nan());
    }
    if y.is_infinite() {
        return Ok(DecValue::infinity(y < 0.0));
    }
    // `float_to_decvalue` gives the EXACT binary value of the float result;
    // rounding THAT to the context is what a correctly-rounded decimal
    // transcendental has to satisfy (precision + Inexact/Rounded flags).
    apply_context(ctx, float_to_decvalue(y))
}

pub fn decimal_ln(ctx: &PyObjectRef, v: &DecValue) -> PyResult<DecValue> {
    if v.special == DecSpecial::Finite && v.sign && !v.is_zero() {
        return Err(decimal_invalid_op("ln"));
    }
    transcendental(ctx, v, "ln")
}

pub fn decimal_log10(ctx: &PyObjectRef, v: &DecValue) -> PyResult<DecValue> {
    if v.special == DecSpecial::Finite && v.sign && !v.is_zero() {
        return Err(decimal_invalid_op("log10"));
    }
    transcendental(ctx, v, "log10")
}

pub fn decimal_logb(ctx: &PyObjectRef, v: &DecValue) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite {
        return Err(decimal_invalid_op("logb"));
    }
    if v.is_zero() {
        return Ok(DecValue::nan());
    }
    let adjusted = decimal_adjusted(v)?;
    Ok(apply_context(ctx, dec_from_int(adjusted))?)
}

pub fn decimal_exp(ctx: &PyObjectRef, v: &DecValue) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite {
        return Err(decimal_invalid_op("exp"));
    }
    transcendental(ctx, v, "exp")
}

/// `Decimal.sqrt()` with context rounding.
pub fn decimal_sqrt_ctx(ctx: &PyObjectRef, v: &DecValue) -> PyResult<DecValue> {
    if v.special == DecSpecial::SNaN {
        return Err(decimal_invalid_op("sqrt"));
    }
    if v.special != DecSpecial::Finite {
        return Err(decimal_invalid_op("sqrt"));
    }
    if v.sign && !v.is_zero() {
        return Err(decimal_invalid_op("sqrt"));
    }
    if v.is_zero() {
        return Ok(v.clone());
    }
    transcendental(ctx, v, "sqrt")
}

/// Coefficient digit helpers for the logical operations.
fn coefficient_digits(v: &DecValue) -> Vec<u8> {
    if v.special != DecSpecial::Finite {
        return vec![0];
    }
    let digits = digit_count(&v.coeff).max(1);
    format!("{:0width$}", v.coeff, width = digits)
        .bytes()
        .map(|b| b - b'0')
        .collect()
}

fn digits_to_value(sign: bool, digits: &[u8], exp: i64) -> DecValue {
    let mut text = String::new();
    for d in digits {
        text.push((b'0' + d) as char);
    }
    let coeff: BigInt = if text.is_empty() {
        BigInt::zero()
    } else {
        text.parse().unwrap_or_else(|_| BigInt::zero())
    };
    DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff,
        exp,
    }
}

/// `logical_and/or/xor/invert` — coefficient-digit-wise, result is always
/// positive with exponent 0.
pub fn decimal_logical(
    ctx: &PyObjectRef,
    a: &DecValue,
    b: Option<&DecValue>,
    op: &str,
) -> PyResult<DecValue> {
    if a.special != DecSpecial::Finite {
        return Err(decimal_invalid_op(op));
    }
    if let Some(other) = b {
        if other.special != DecSpecial::Finite {
            return Err(decimal_invalid_op(op));
        }
    }
    let da = coefficient_digits(a);
    let db = b.map(coefficient_digits).unwrap_or_default();
    let width = da.len().max(db.len());
    let mut out = Vec::with_capacity(width);
    for i in 0..width {
        let x = *da.get(i).unwrap_or(&0);
        let y = *db.get(i).unwrap_or(&0);
        out.push(match op {
            "and" => x & y,
            "or" => x | y,
            "xor" => x ^ y,
            _ => 9 - x,
        });
    }
    let _ = ctx;
    Ok(digits_to_value(false, &out, 0))
}

/// `number_class()` — CPython's classification string.
pub fn decimal_number_class(v: &DecValue) -> &'static str {
    match v.special {
        DecSpecial::QNaN | DecSpecial::SNaN => "NaN",
        DecSpecial::Infinity => "Infinity",
        DecSpecial::Finite => {
            if v.is_zero() {
                "Zero"
            } else if v.sign {
                "Negative"
            } else {
                "Positive"
            }
        }
    }
}

/// `to_eng_string()` — engineering notation. CPython prints the plain
/// (non-exponential) form whenever the adjusted exponent is already a
/// multiple of three, and aligns to the next lower multiple of three
/// otherwise: `Decimal('12300').to_eng_string() == '12300'`,
/// `Decimal('1.23E5').to_eng_string() == '123E+3'`.
pub fn decimal_to_eng_string(v: &DecValue) -> String {
    match v.special {
        DecSpecial::Infinity => {
            return if v.sign { "-Infinity" } else { "Infinity" }.to_string();
        }
        DecSpecial::QNaN => return "NaN".to_string(),
        DecSpecial::SNaN => return "sNaN".to_string(),
        DecSpecial::Finite => {}
    }
    if v.is_zero() {
        return if v.sign { "-0" } else { "0" }.to_string();
    }
    // Engineering notation moves the exponent to the next multiple of three
    // AWAY from zero, and is only used when the mantissa still fits in three
    // digits and the value's magnitude is above one (all verified against
    // CPython 3.14: `12300` -> `12300`, `1.23E4` -> `12.3E+3`,
    // `1.23E5` -> `123E+3`, `1.23E6` -> `1.23E+6`,
    // `0.0000123` -> `0.0000123`).
    let digits: String = format!("{}", v.coeff);
    let ndigits = digits.len() as i64;
    let exponent = v.exp;
    let adjusted = exponent + ndigits - 1;
    let away = if exponent > 0 {
        exponent + (3 - exponent.rem_euclid(3)).rem_euclid(3)
    } else {
        exponent - ((-exponent).rem_euclid(3))
    };
    if away == 0 || ndigits > 3 || adjusted <= 0 {
        return format_decvalue(v);
    }
    if ndigits > 3 {
        return format_decvalue(v);
    }
    // Digits before the point: `ndigits + exponent - away`.
    let point = ndigits + exponent - away;
    let mut out = String::new();
    if v.sign {
        out.push('-');
    }
    if point <= 0 {
        out.push_str("0.");
        for _ in 0..(-point) {
            out.push('0');
        }
        out.push_str(&digits);
    } else if point as usize >= digits.len() {
        out.push_str(&digits);
        for _ in 0..(point as usize - digits.len()) {
            out.push('0');
        }
    } else {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    }
    out.push_str(&format!("E+{}", away));
    out
}

/// `to_sci_string()` — scientific notation.
pub fn decimal_to_sci_string(v: &DecValue) -> String {
    let formatted = format_decvalue(v);
    match formatted.find(['e', 'E']) {
        Some(pos) => {
            let (m, e) = formatted.split_at(pos);
            let exp: i64 = e[1..].parse().unwrap_or(0);
            format!("{}E{}", m, if exp < 0 { exp } else { exp })
        }
        None => {
            let digits: String = formatted.chars().filter(|c| c.is_ascii_digit()).collect();
            let point = formatted.find('.').unwrap_or(formatted.len());
            let exp = point as i64 - 1;
            let mut mantissa = String::new();
            let negative = formatted.starts_with('-');
            if negative {
                mantissa.push('-');
            }
            mantissa.push_str(&digits[..digits.len().min(1)]);
            if digits.len() > 1 {
                mantissa.push('.');
                mantissa.push_str(&digits[1..]);
            }
            format!("{}E{}", mantissa, exp)
        }
    }
}

/// Read the value of a `Decimal` argument for a Context method.
///
/// CPython's `Context` methods accept only Decimal, int and bool — NOT
/// str/bytes/tuple (`c.add('1.2', d)` is a TypeError) and NOT float (since
/// 3.2, mixing Decimals and floats is a TypeError unless the
/// `FloatOperation` trap is disabled, which is what
/// `c.fma(Decimal(1), Decimal('snan'), 1.222)` reports).
pub fn ctx_decimal_arg(v: &PyObjectRef) -> PyResult<DecValue> {
    match &*v.borrow() {
        PyObject::Str(_)
        | PyObject::Bytes(_)
        | PyObject::Tuple(_)
        | PyObject::List(_)
        | PyObject::Float(_) => Err(PyError::type_error(format!(
            "conversion from {} to Decimal is not supported",
            v.get_type_name()
        ))),
        _ => decval_from_pyobject(v),
    }
}

/// Build the `Context` type with CPython's full method surface.
pub fn build_context_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            type_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $f,
                }),
            );
        };
    }
    macro_rules! prop {
        ($name:expr, $key:expr) => {
            type_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                    getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                        name: $name.to_string(),
                        func: move |args: &[PyObjectRef]| {
                            let value = match &*args[0].borrow() {
                                PyObject::Instance { dict, .. } => dict.get_str($key).cloned(),
                                _ => None,
                            };
                            Ok(value.unwrap_or_else(py_none))
                        },
                    })),
                    setter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                        name: format!("{}_setter", $name),
                        func: move |args: &[PyObjectRef]| {
                            let value = args.get(1).cloned().unwrap_or_else(py_none);
                            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                                dict.insert_str($key, value);
                            }
                            if $key == "prec" {
                                ctx_update_etiny(&args[0]);
                            }
                            Ok(py_none())
                        },
                    })),
                    deleter: None,
                    doc: None,
                }))),
            );
        };
    }
    prop!("prec", "prec");
    prop!("rounding", "rounding");
    // CPython exposes the signal bitmasks as `_flags`/`_traps`;
    // test_decimal's `test_construction` builds a Context with
    // `flags=[...]`/`traps=[...]` and then checks exactly these integers.
    for name in ["_flags", "_traps"] {
        let is_flags = name == "_flags";
        type_dict.insert_str(
            name,
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                    move |args: &[PyObjectRef]| {
                        let kind = if is_flags { "flags" } else { "traps" };
                        let mut bits = 0i64;
                        for (i, signal) in SIGNALS.iter().enumerate() {
                            if read_signal(&args[0], kind, signal)
                                .map(|v| v.truthy())
                                .unwrap_or(false)
                            {
                                bits |= 1 << i;
                            }
                        }
                        Ok(py_int(bits))
                    },
                )))),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }
    prop!("Emin", "Emin");
    prop!("Emax", "Emax");
    prop!("Etiny", "Etiny");
    prop!("Etop", "Etop");
    prop!("capitals", "capitals");
    prop!("clamp", "clamp");
    type_dict.insert_str(
        "flags",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "flags".to_string(),
                func: |args: &[PyObjectRef]| {
                    args[0]
                        .borrow()
                        .get_attribute("flags")
                        .map_err(|_| PyError::runtime_error("no flags"))
                },
            })),
            setter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "flags_setter".to_string(),
                func: |args: &[PyObjectRef]| {
                    let value = args.get(1).cloned().unwrap_or_else(py_none);
                    if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                        dict.insert_str("flags", value);
                    }
                    Ok(py_none())
                },
            })),
            deleter: None,
            doc: None,
        }))),
    );
    type_dict.insert_str(
        "traps",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "traps".to_string(),
                func: |args: &[PyObjectRef]| {
                    args[0]
                        .borrow()
                        .get_attribute("traps")
                        .map_err(|_| PyError::runtime_error("no traps"))
                },
            })),
            setter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "traps_setter".to_string(),
                func: |args: &[PyObjectRef]| {
                    let value = args.get(1).cloned().unwrap_or_else(py_none);
                    if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                        dict.insert_str("traps", value);
                    }
                    Ok(py_none())
                },
            })),
            deleter: None,
            doc: None,
        }))),
    );
    bf!("__init__", |args: &[PyObjectRef]| {
        if args.is_empty() {
            return Err(PyError::type_error("Context() missing self"));
        }
        let ctx = args[0].clone();
        let mut kw: HashMap<String, PyObjectRef> = HashMap::new();
        let mut positional: Vec<PyObjectRef> = Vec::new();
        for arg in &args[1..] {
            match &*arg.borrow() {
                PyObject::Dict(d) => {
                    for (k, v) in d.iter() {
                        kw.insert(k.str(), v.clone());
                    }
                }
                _ => positional.push(arg.clone()),
            }
        }
        // Positional order: prec, rounding, Emin, Emax, capitals, clamp.
        let names = ["prec", "rounding", "Emin", "Emax", "capitals", "clamp"];
        for (i, value) in positional.iter().enumerate() {
            if i < names.len() {
                kw.entry(names[i].to_string()).or_insert(value.clone());
            }
        }
        // `Context(prec=None, rounding=None, ...)`: an explicit None means
        // "use the default", exactly like omitting the argument.
        kw.retain(|_, v| !matches!(&*v.borrow(), PyObject::None));
        let existing = ctx_prec(&ctx) as i64;
        let prec = kw.get("prec").and_then(|v| v.as_i64()).unwrap_or(existing);
        let rounding = kw
            .get("rounding")
            .map(|v| v.str())
            .unwrap_or_else(|| ctx_rounding(&ctx));
        let emin = kw
            .get("Emin")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| ctx_int_field(&ctx, "Emin", -999999));
        let emax = kw
            .get("Emax")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| ctx_int_field(&ctx, "Emax", 999999));
        let capitals = kw.get("capitals").map(|v| v.truthy()).unwrap_or_else(|| {
            ctx_dict_field(&ctx, "capitals")
                .map(|v| v.truthy())
                .unwrap_or(true)
        });
        let clamp = kw.get("clamp").map(|v| v.truthy()).unwrap_or_else(|| {
            ctx_dict_field(&ctx, "clamp")
                .map(|v| v.truthy())
                .unwrap_or(false)
        });
        ctx_set_field(&ctx, "prec", py_int(prec));
        ctx_set_field(&ctx, "rounding", py_str(&rounding));
        ctx_set_field(&ctx, "Emin", py_int(emin));
        ctx_set_field(&ctx, "Emax", py_int(emax));
        ctx_set_field(&ctx, "capitals", py_bool(capitals));
        ctx_set_field(&ctx, "clamp", py_bool(clamp));
        ctx_update_etiny(&ctx);
        // The containers must exist BEFORE the `flags=`/`traps=` kwargs are
        // applied — a bare `Context(...)` starts as an instance with an
        // empty dict, and `write_signal` silently does nothing when the
        // container is missing.
        if ctx_dict_field(&ctx, "flags").is_none() {
            let flags = make_signal_container(&ctx, "flags");
            ctx_set_field(&ctx, "flags", flags);
            let traps = make_signal_container(&ctx, "traps");
            ctx_set_field(&ctx, "traps", traps);
            for name in SIGNALS {
                write_signal(&ctx, "flags", name, false);
                // A fresh `Context()` uses CPython's default trap set
                // (InvalidOperation, DivisionByZero, Overflow), not just
                // InvalidOperation.
                write_signal(&ctx, "traps", name, DEFAULT_TRAPS.contains(&name));
            }
        }
        // `flags=`/`traps=` accept any iterable of signal objects (or
        // names); CPython's test_construction passes `list(flags)`.
        for key in ["flags", "traps"] {
            if let Some(value) = kw.get(key) {
                let items: Vec<PyObjectRef> = match &*value.borrow() {
                    PyObject::List(l) => l.clone(),
                    PyObject::Tuple(t) => t.clone(),
                    PyObject::Set(s) => s.to_vec(),
                    _ => vec![value.clone()],
                };
                for name in SIGNALS {
                    write_signal(&ctx, key, name, false);
                }
                for item in items {
                    if let Some(signal) = signal_name_of(&item) {
                        write_signal(&ctx, key, &signal, true);
                    }
                }
            }
        }
        Ok(py_none())
    });
    bf!("__repr__", |args: &[PyObjectRef]| {
        let ctx = args
            .first()
            .ok_or_else(|| PyError::type_error("__repr__() missing context"))?;
        let get = |name: &str| -> String {
            ctx_dict_field(ctx, name)
                .map(|v| v.str())
                .unwrap_or_default()
        };
        Ok(py_str(&format!(
            "<Context [{}]>",
            format!(
                "prec={} rounding={} Emin={} Emax={} capitals={} clamp={}",
                get("prec"),
                get("rounding"),
                get("Emin"),
                get("Emax"),
                ctx_dict_field(ctx, "capitals")
                    .map(|v| if v.truthy() { 1 } else { 0 })
                    .unwrap_or(1),
                ctx_dict_field(ctx, "clamp")
                    .map(|v| if v.truthy() { 1 } else { 0 })
                    .unwrap_or(0),
            )
        )))
    });
    bf!("copy", |args: &[PyObjectRef]| {
        let ctx = match args.first() {
            Some(c) => c,
            None => return Err(PyError::type_error("copy() missing context")),
        };
        let copy = new_context(
            ctx_prec(ctx) as i64,
            &ctx_rounding(ctx),
            ctx_int_field(ctx, "Emin", -999999),
            ctx_int_field(ctx, "Emax", 999999),
            ctx_dict_field(ctx, "capitals")
                .map(|v| v.truthy())
                .unwrap_or(true),
            ctx_dict_field(ctx, "clamp")
                .map(|v| v.truthy())
                .unwrap_or(false),
            &[],
        );
        for name in SIGNALS {
            let traps = read_signal(ctx, "traps", name).unwrap_or_else(|_| py_bool(false));
            let flags = read_signal(ctx, "flags", name).unwrap_or_else(|_| py_bool(false));
            write_signal(&copy, "traps", name, traps.truthy());
            write_signal(&copy, "flags", name, flags.truthy());
        }
        Ok(copy)
    });
    bf!("__copy__", |args: &[PyObjectRef]| {
        let ctx = match args.first() {
            Some(c) => c.clone(),
            None => return Err(PyError::type_error("__copy__() missing context")),
        };
        // `copy` is looked up as a plain function, so `self` has to be
        // passed explicitly (calling it with no arguments panicked on the
        // missing `args[0]`).
        let copy = ctx
            .borrow()
            .get_attribute("copy")
            .map_err(|_| PyError::runtime_error("no copy"))?;
        crate::object::builtin_call(&copy, &[ctx])
    });
    bf!("__deepcopy__", |args: &[PyObjectRef]| {
        let ctx = args
            .first()
            .cloned()
            .ok_or_else(|| PyError::type_error("__deepcopy__() missing context"))?;
        let copy = ctx
            .borrow()
            .get_attribute("copy")
            .map_err(|_| PyError::runtime_error("no copy"))?;
        crate::object::builtin_call(&copy, &[ctx])
    });
    bf!("clear_flags", |args: &[PyObjectRef]| {
        let ctx = args
            .first()
            .ok_or_else(|| PyError::type_error("clear_flags() missing context"))?;
        for name in SIGNALS {
            write_signal(ctx, "flags", name, false);
        }
        Ok(py_none())
    });
    bf!("clear_traps", |args: &[PyObjectRef]| {
        let ctx = args
            .first()
            .ok_or_else(|| PyError::type_error("clear_traps() missing context"))?;
        for name in SIGNALS {
            write_signal(ctx, "traps", name, false);
        }
        Ok(py_none())
    });
    bf!("create_decimal", |args: &[PyObjectRef]| {
        if args.len() < 2 {
            return Err(PyError::type_error("create_decimal() missing argument"));
        }
        let ctx = args[0].clone();
        let value = ctx_decimal_arg(&args[1])?;
        let out = apply_context(&ctx, value)?;
        Ok(decval_to_instance(&out))
    });
    bf!("create_decimal_from_float", |args: &[PyObjectRef]| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "create_decimal_from_float() missing argument",
            ));
        }
        let ctx = args[0].clone();
        let f = match &*args[1].borrow() {
            PyObject::Float(f) => *f,
            other => {
                return Err(PyError::type_error(format!(
                    "conversion from {} to Decimal is not supported",
                    other.type_name()
                )))
            }
        };
        if f.is_nan() {
            return Err(decimal_signal_error(
                "FloatOperation",
                "[<class 'float'>] either does not support arithmetic operations or the operation is invalid",
            ));
        }
        let out = apply_context(&ctx, float_to_decvalue(f))?;
        Ok(decval_to_instance(&out))
    });
    bf!("radix", |_args: &[PyObjectRef]| Ok(py_int(10)));
    bf!("copy_decimal", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let out = apply_context(&ctx, a)?;
        Ok(decval_to_instance(&out))
    });
    bf!("next_plus", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_next(&ctx, &a, 1)?))
    });
    bf!("next_minus", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_next(&ctx, &a, -1)?))
    });
    bf!("next_toward", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        Ok(decval_to_instance(&decimal_next_toward(&a, &b)?))
    });

    // ---- arithmetic -------------------------------------------------
    macro_rules! ctx_op {
        ($name:expr, $f:expr) => {
            bf!($name, $f);
        };
    }
    ctx_op!("add", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = apply_context(&ctx, decimal_add(&a, &b)?)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("subtract", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = apply_context(&ctx, decimal_sub(&a, &b)?)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("multiply", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = apply_context(&ctx, decimal_mul(&a, &b)?)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("divide", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = apply_context(&ctx, decimal_div(&a, &b)?)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("divide_int", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        if b.is_zero() {
            check_signal(&ctx, "DivisionByZero", "Division by zero")?;
        }
        if b.special != DecSpecial::Finite {
            return Err(decimal_invalid_op("divide_int"));
        }
        let one = dec_from_int(1);
        let q = decimal_trunc_quotient(&a, &b)?;
        let r = decimal_sub(&a, &decimal_mul(&q, &b)?)?;
        let _ = one;
        let out = apply_context(&ctx, q)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("power", |args: &[PyObjectRef]| {
        // CPython's signature is `power(a, b, modulo=None)`.
        let values = ctx_named(args, &["a", "b"])?;
        let ctx = args
            .first()
            .cloned()
            .ok_or_else(|| PyError::type_error("power() missing self"))?;
        let a = ctx_decimal_arg(&values[0])?;
        let b = ctx_decimal_arg(&values[1])?;
        // `modulo` is the optional third parameter (also keyword-only in
        // practice: `c.power(modulo=5, b=8, a=2)`).
        let modulo = args
            .iter()
            .skip(1)
            .filter_map(|v| match &*v.borrow() {
                PyObject::Dict(d) => d.get(&py_str("modulo")).ok().flatten(),
                _ => None,
            })
            .next();
        let mut out = apply_context(&ctx, decimal_pow(&a, &b)?)?;
        if let Some(m) = modulo {
            let m = ctx_decimal_arg(&m)?;
            if m.is_zero() {
                return Err(decimal_invalid_op("power"));
            }
            out = decimal_remainder(&out, &m)?;
            out = apply_context(&ctx, out)?;
        }
        Ok(decval_to_instance(&out))
    });
    ctx_op!("mod", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = apply_context(&ctx, decimal_remainder(&a, &b)?)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("remainder", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = apply_context(&ctx, decimal_remainder(&a, &b)?)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("divmod", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let q = apply_context(&ctx, decimal_trunc_quotient(&a, &b)?)?;
        let r = apply_context(&ctx, decimal_remainder(&a, &b)?)?;
        Ok(PyObjectRef::new(PyObject::Tuple(vec![
            decval_to_instance(&q),
            decval_to_instance(&r),
        ])))
    });
    ctx_op!("plus", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let out = apply_context(&ctx, a.clone())?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("minus", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let out = apply_context(&ctx, decimal_negate(&a))?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("abs", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let out = apply_context(&ctx, decimal_copy_abs(&a))?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("negate", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let out = apply_context(&ctx, decimal_negate(&a))?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("copy_abs", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_copy_abs(&a)))
    });
    ctx_op!("copy_negate", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_negate(&a)))
    });
    ctx_op!("copy_sign", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        Ok(decval_to_instance(&decimal_copy_sign(&a, &b)))
    });
    ctx_op!("exp", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_exp(&ctx, &a)?))
    });
    ctx_op!("ln", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_ln(&ctx, &a)?))
    });
    ctx_op!("log10", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_log10(&ctx, &a)?))
    });
    ctx_op!("logb", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_logb(&ctx, &a)?))
    });
    ctx_op!("sqrt", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_sqrt_ctx(&ctx, &a)?))
    });
    ctx_op!("fma", |args: &[PyObjectRef]| {
        let ctx = args[0].clone();
        let a = ctx_decimal_arg(&args[1])?;
        let b = ctx_decimal_arg(&args[2])?;
        let c = ctx_decimal_arg(&args[3])?;
        Ok(decval_to_instance(&decimal_fma(&ctx, &a, &b, &c)?))
    });
    ctx_op!("quantize", |args: &[PyObjectRef]| {
        let values = ctx_named(args, &["x", "y"])?;
        let ctx = args
            .first()
            .cloned()
            .ok_or_else(|| PyError::type_error("quantize() missing self"))?;
        let a = ctx_decimal_arg(&values[0])?;
        let exp = ctx_exp_arg(&values[1])?;
        let rounding = ctx_rounding_arg(args.iter().rev().find(|a| {
            !matches!(&*a.borrow(), PyObject::Instance { .. })
                && !matches!(&*a.borrow(), PyObject::Str(_) | PyObject::Int(_))
        }))?;
        let out = decimal_quantize(&ctx, &a, exp, rounding.as_deref())?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("to_integral", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let rounding = ctx_rounding_arg(args.get(2))?;
        let out = decimal_to_integral_exact(&ctx, &a, rounding.as_deref())?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("to_integral_value", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let rounding = ctx_rounding_arg(args.get(2))?;
        let out = decimal_to_integral_value(&ctx, &a, rounding.as_deref())?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("to_integral_exact", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let rounding = ctx_rounding_arg(args.get(2))?;
        let out = decimal_to_integral_exact(&ctx, &a, rounding.as_deref())?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("scaleb", |args: &[PyObjectRef]| {
        let (ctx, a, other) = ctx_two_args(args)?;
        // The second operand is an INTEGER (a Decimal with an integral value
        // is accepted too); anything else is a TypeError.
        let n = match other
            .coeff
            .to_i64()
            .filter(|_| other.special == DecSpecial::Finite)
        {
            Some(n) => {
                if other.sign {
                    -n
                } else {
                    n
                }
            }
            None => return Err(PyError::type_error("scaleb() requires an integer")),
        };
        Ok(decval_to_instance(&decimal_scaleb(&ctx, &a, n)?))
    });
    ctx_op!("shift", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        Ok(decval_to_instance(&decimal_shift(&ctx, &a, &b)?))
    });
    ctx_op!("rotate", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        Ok(decval_to_instance(&decimal_rotate(&a, &b)?))
    });
    ctx_op!("normalize", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        let out = apply_context(&ctx, normalize_decval(&a))?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("remainder_near", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        Ok(decval_to_instance(&decimal_remainder_near(&a, &b)?))
    });
    ctx_op!("to_eng_string", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_str(&decimal_to_eng_string(&a)))
    });
    ctx_op!("to_sci_string", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        // CPython's `Context.to_sci_string(x)` is the plain string form.
        Ok(py_str(&format_decvalue(&a)))
    });
    ctx_op!("number_class", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_str(decimal_number_class(&a)))
    });
    // ---- comparisons ------------------------------------------------
    ctx_op!("compare", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        decimal_compare(&a, &b)
            .map(|o| {
                py_int(match o {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                })
            })
            .ok_or_else(|| decimal_invalid_op("compare"))
    });
    ctx_op!("compare_signal", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        decimal_compare(&a, &b)
            .map(|o| {
                py_int(match o {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                })
            })
            .ok_or_else(|| decimal_invalid_op("compare_signal"))
    });
    ctx_op!("compare_total", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        Ok(py_int(decimal_compare_total(&a, &b)))
    });
    ctx_op!("compare_total_mag", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        Ok(py_int(decimal_compare_total_mag(&a, &b)))
    });
    ctx_op!("max", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = if decimal_compare(&a, &b) == Some(std::cmp::Ordering::Less) {
            b
        } else {
            a
        };
        let out = apply_context(&ctx, out)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("min", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = if decimal_compare(&a, &b) == Some(std::cmp::Ordering::Greater) {
            b
        } else {
            a
        };
        let out = apply_context(&ctx, out)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("max_mag", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = if decimal_compare_total_mag(&a, &b) < 0 {
            b
        } else {
            a
        };
        let out = apply_context(&ctx, out)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("min_mag", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        let out = if decimal_compare_total_mag(&a, &b) > 0 {
            b
        } else {
            a
        };
        let out = apply_context(&ctx, out)?;
        Ok(decval_to_instance(&out))
    });
    ctx_op!("same_quantum", |args: &[PyObjectRef]| {
        let (_, a, b) = ctx_two_args(args)?;
        Ok(py_bool(decimal_same_quantum(&a, &b)))
    });
    ctx_op!("is_canonical", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_bool(decimal_is_canonical(&a)))
    });
    ctx_op!("is_finite", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_bool(a.special == DecSpecial::Finite))
    });
    ctx_op!("is_infinite", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_bool(a.special == DecSpecial::Infinity))
    });
    ctx_op!("is_nan", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_bool(a.is_nan()))
    });
    ctx_op!("is_normal", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(py_bool(decimal_is_normal(&ctx, &a)))
    });
    ctx_op!("is_qnan", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_bool(a.special == DecSpecial::QNaN))
    });
    ctx_op!("is_snan", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_bool(a.special == DecSpecial::SNaN))
    });
    ctx_op!("is_subnormal", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(py_bool(decimal_is_subnormal(&ctx, &a)))
    });
    ctx_op!("is_signed", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_bool(a.sign))
    });
    ctx_op!("is_zero", |args: &[PyObjectRef]| {
        let (_, a) = ctx_one_arg(args)?;
        Ok(py_bool(a.is_zero()))
    });
    // ---- logical ----------------------------------------------------
    ctx_op!("logical_and", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        Ok(decval_to_instance(&decimal_logical(
            &ctx,
            &a,
            Some(&b),
            "and",
        )?))
    });
    ctx_op!("logical_or", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        Ok(decval_to_instance(&decimal_logical(
            &ctx,
            &a,
            Some(&b),
            "or",
        )?))
    });
    ctx_op!("logical_xor", |args: &[PyObjectRef]| {
        let (ctx, a, b) = ctx_two_args(args)?;
        Ok(decval_to_instance(&decimal_logical(
            &ctx,
            &a,
            Some(&b),
            "xor",
        )?))
    });
    ctx_op!("logical_invert", |args: &[PyObjectRef]| {
        let (ctx, a) = ctx_one_arg(args)?;
        Ok(decval_to_instance(&decimal_logical(
            &ctx, &a, None, "invert",
        )?))
    });
    PyObjectRef::new(PyObject::Type {
        name: "Context".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

/// `(self, a)` for one-operand context methods.
fn ctx_one_arg(args: &[PyObjectRef]) -> PyResult<(PyObjectRef, DecValue)> {
    let values = ctx_named(args, &["x"])?;
    let ctx = args
        .first()
        .cloned()
        .ok_or_else(|| PyError::type_error("context method missing self"))?;
    let a = ctx_decimal_arg(&values[0])?;
    Ok((ctx, a))
}

/// `(self, a, b)` for two-operand context methods.
fn ctx_two_args(args: &[PyObjectRef]) -> PyResult<(PyObjectRef, DecValue, DecValue)> {
    let values = ctx_named(args, &["x", "y"])?;
    let ctx = args
        .first()
        .cloned()
        .ok_or_else(|| PyError::type_error("context method missing self"))?;
    let a = ctx_decimal_arg(&values[0])?;
    let b = ctx_decimal_arg(&values[1])?;
    Ok((ctx, a, b))
}

/// Resolve a context method's arguments by position OR keyword. CPython's
/// Context methods are keyword-callable (`c.power(a=2, b=8, modulo=5)`), and
/// this project's calling convention delivers keywords as a trailing
/// `PyDict`, so both shapes have to be accepted.
fn ctx_named(args: &[PyObjectRef], names: &[&str]) -> PyResult<Vec<PyObjectRef>> {
    // The call machinery appends the keyword dict even when it is empty.
    let has_kwargs =
        matches!(args.last(), Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)));
    let (positional, kwargs) = if has_kwargs {
        (&args[..args.len() - 1], args.last())
    } else {
        (args, None)
    };
    // Drop `self` when it is a Context instance.
    let positional: &[PyObjectRef] = match positional.first() {
        Some(first) if matches!(&*first.borrow(), PyObject::Instance { .. }) => &positional[1..],
        _ => positional,
    };
    if positional.len() > names.len() {
        if std::env::var("RPY_DEBUG_DEC_ARGS").is_ok() {
            eprintln!(
                "ctx_named {:?} positional={:?} kwargs={}",
                names,
                positional.iter().map(|a| a.repr()).collect::<Vec<_>>(),
                kwargs.map(|k| k.repr()).unwrap_or_default()
            );
        }
        return Err(PyError::type_error("too many arguments"));
    }
    let mut out = Vec::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        let value = positional.get(i).cloned().or_else(|| {
            kwargs.and_then(|kw| match &*kw.borrow() {
                PyObject::Dict(d) => d.get(&py_str(name)).ok().flatten(),
                _ => None,
            })
        });
        out.push(value.ok_or_else(|| PyError::type_error(format!("missing argument '{}'", name)))?);
    }
    Ok(out)
}

/// The exponent argument of `quantize`: for a `Decimal` it is the argument's
/// EXPONENT (`quantize(Decimal('0.01'))` means exponent -2), and for an int
/// the value itself.
fn ctx_exp_arg(obj: &PyObjectRef) -> PyResult<i64> {
    // An int argument is converted like `Decimal(int)`, whose exponent is 0:
    // CPython's `c.quantize(Decimal('12345'), 2)` rounds to exponent 0 (and
    // leaves the value alone), it does NOT round to hundreds.
    if let Some(n) = obj.as_i64() {
        let _ = n;
        return Ok(0);
    }
    let value = ctx_decimal_arg(obj)?;
    if value.special != DecSpecial::Finite {
        return Err(decimal_invalid_op("quantize"));
    }
    Ok(value.exp)
}

/// `quantize(exp, rounding=None)` — rounds to the requested exponent,
/// rejecting results that would exceed the context precision (CPython
/// raises InvalidOperation for those, it does not silently round).
pub fn decimal_quantize(
    ctx: &PyObjectRef,
    v: &DecValue,
    exp: i64,
    rounding: Option<&str>,
) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite {
        if v.special == DecSpecial::Infinity {
            return Ok(v.clone());
        }
        return Err(decimal_invalid_op("quantize"));
    }
    let default_rounding = ctx_rounding(ctx);
    let rounding = rounding.unwrap_or(default_rounding.as_str());
    if exp < ctx_int_field(ctx, "Etiny", -1000000) {
        return Err(decimal_invalid_op("quantize"));
    }
    let digits = digit_count(&v.coeff) as i64;
    let adjusted = if v.is_zero() {
        v.exp
    } else {
        v.exp + digits - 1
    };
    if adjusted > ctx_int_field(ctx, "Emax", 999999) {
        return Err(decimal_invalid_op("quantize"));
    }
    // Already representable at the requested exponent: CPython keeps the
    // operand's own exponent (`c.quantize(Decimal(1), Decimal(2))` is
    // `Decimal('1')`, not `Decimal('0E+2')`).
    if v.exp > exp && digit_count(&v.coeff) as i64 + (v.exp - exp) <= ctx_prec(ctx) as i64 {
        return Ok(v.clone());
    }
    let out = round_to_exponent(v, exp, rounding);
    // The RESULT must fit the context precision: quantize never rounds a
    // value that would need more digits than the context allows.
    if digit_count(&out.coeff) > ctx_prec(ctx) {
        return Err(decimal_invalid_op("quantize"));
    }
    Ok(out)
}

/// The optional `rounding=` argument of the integral/quantize family: a
/// rounding-mode STRING (or nothing). CPython raises TypeError for anything
/// else, e.g. `c.to_integral_value(10)`.
pub fn ctx_rounding_arg(obj: Option<&PyObjectRef>) -> PyResult<Option<String>> {
    match obj {
        None => Ok(None),
        Some(v) => {
            if matches!(&*v.borrow(), PyObject::Str(_)) {
                Ok(Some(v.str()))
            } else {
                Err(PyError::type_error(format!(
                    "rounding must be a string, not {}",
                    v.get_type_name()
                )))
            }
        }
    }
}

/// `to_integral_value(rounding=None)` — rounds to exponent 0, signalling
/// Inexact/Rounded when digits were dropped.
pub fn decimal_to_integral_value(
    ctx: &PyObjectRef,
    v: &DecValue,
    rounding: Option<&str>,
) -> PyResult<DecValue> {
    if v.special != DecSpecial::Finite {
        return Ok(v.clone());
    }
    let default_rounding = ctx_rounding(ctx);
    let rounding = rounding.unwrap_or(default_rounding.as_str());
    if v.exp >= 0 {
        return Ok(v.clone());
    }
    check_signal(ctx, "Rounded", "Rounded")?;
    check_signal(ctx, "Inexact", "Inexact")?;
    Ok(round_to_exponent(v, 0, rounding))
}

/// `to_integral_exact(rounding=None)` — CPython signals the same
/// conditions as `to_integral_value` here.
pub fn decimal_to_integral_exact(
    ctx: &PyObjectRef,
    v: &DecValue,
    rounding: Option<&str>,
) -> PyResult<DecValue> {
    decimal_to_integral_value(ctx, v, rounding)
}

/// `copy_abs()`.
pub fn decimal_copy_abs(v: &DecValue) -> DecValue {
    DecValue {
        sign: false,
        ..v.clone()
    }
}

/// `copy_sign(other)`.
pub fn decimal_copy_sign(v: &DecValue, other: &DecValue) -> DecValue {
    DecValue {
        sign: other.sign,
        ..v.clone()
    }
}

/// Truncating quotient shared with the Decimal dunders.
pub fn decimal_trunc_quotient(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    let q = decimal_div(a, b)?;
    if q.special != DecSpecial::Finite {
        return Ok(q);
    }
    let truncated = if q.exp >= 0 {
        &q.coeff * ten_pow(q.exp)
    } else {
        &q.coeff / ten_pow(-q.exp)
    };
    Ok(DecValue {
        special: DecSpecial::Finite,
        sign: q.sign,
        coeff: truncated,
        exp: 0,
    })
}

/// `a % b` with the sign of `a` (Decimal semantics, not Python's).
pub fn decimal_remainder(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if b.is_zero() {
        return Err(decimal_division_by_zero("0 modulo"));
    }
    if a.special != DecSpecial::Finite || b.special != DecSpecial::Finite {
        if a.is_nan() || b.is_nan() {
            return Ok(DecValue::nan());
        }
        if a.special == DecSpecial::Infinity {
            return Err(decimal_invalid_op("remainder"));
        }
        return Ok(DecValue::nan());
    }
    let q = decimal_trunc_quotient(a, b)?;
    let prod = decimal_mul(&q, b)?;
    decimal_sub(a, &prod)
}

/// `a ** b` for integer and fractional exponents.
pub fn decimal_pow(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if b.special != DecSpecial::Finite {
        if b.is_zero() {
            return Ok(dec_from_int(1));
        }
        if a.special == DecSpecial::Finite && !a.is_zero() {
            return Ok(DecValue::infinity(false));
        }
        return Err(decimal_invalid_op("pow"));
    }
    if a.special != DecSpecial::Finite {
        return Ok(a.clone());
    }
    if b.exp >= 0 && b.coeff.bits() <= 32 {
        let mut n = b.coeff.to_u32().unwrap_or(0);
        if b.sign {
            // Negative integer exponent: 1 / a**n
            let mut acc = dec_from_int(1);
            while n > 0 {
                acc = decimal_mul(&acc, a)?;
                n -= 1;
            }
            if acc.is_zero() {
                return Err(decimal_division_by_zero("division by zero"));
            }
            return decimal_div(&dec_from_int(1), &acc);
        }
        let mut acc = dec_from_int(1);
        while n > 0 {
            acc = decimal_mul(&acc, a)?;
            n -= 1;
        }
        return Ok(acc);
    }
    Err(decimal_invalid_op("pow"))
}

/// `compare_total()` — total ordering including NaN placement.
pub fn decimal_compare_total(a: &DecValue, b: &DecValue) -> i64 {
    use std::cmp::Ordering;
    fn rank(v: &DecValue) -> i32 {
        match v.special {
            DecSpecial::SNaN => 0,
            DecSpecial::QNaN => 1,
            DecSpecial::Infinity => {
                if v.sign {
                    2
                } else {
                    6
                }
            }
            DecSpecial::Finite => {
                if v.sign {
                    3
                } else {
                    5
                }
            }
        }
    }
    let (ra, rb) = (rank(a), rank(b));
    if ra != rb {
        return if ra < rb { -1 } else { 1 };
    }
    if a.special != DecSpecial::Finite || b.special != DecSpecial::Finite {
        return 0;
    }
    match decimal_compare(&decimal_copy_abs(a), &decimal_copy_abs(b)) {
        Some(Ordering::Less) => {
            if a.sign {
                1
            } else {
                -1
            }
        }
        Some(Ordering::Greater) => {
            if a.sign {
                -1
            } else {
                1
            }
        }
        _ => {
            // Equal magnitude: the exponent breaks the tie (smaller
            // coefficient exponent sorts first for equal values).
            match a.exp.cmp(&b.exp) {
                Ordering::Equal => 0,
                Ordering::Less => -1,
                Ordering::Greater => 1,
            }
        }
    }
}

/// `compare_total_mag()` — magnitude-only total ordering.
pub fn decimal_compare_total_mag(a: &DecValue, b: &DecValue) -> i64 {
    decimal_compare_total(&decimal_copy_abs(a), &decimal_copy_abs(b))
}

/// `same_quantum()`.
pub fn decimal_same_quantum(a: &DecValue, b: &DecValue) -> bool {
    if a.is_zero() && b.is_zero() {
        return a.exp == b.exp;
    }
    if a.is_zero() || b.is_zero() {
        let (zero, other) = if a.is_zero() { (a, b) } else { (b, a) };
        return other.exp <= zero.exp;
    }
    a.exp == b.exp
}

/// `is_canonical()`.
pub fn decimal_is_canonical(v: &DecValue) -> bool {
    v.special != DecSpecial::Finite || !v.is_zero() || v.coeff == BigInt::zero()
}

/// `is_normal()` — finite, non-zero, adjusted exponent within range.
pub fn decimal_is_normal(ctx: &PyObjectRef, v: &DecValue) -> bool {
    if v.special != DecSpecial::Finite || v.is_zero() {
        return false;
    }
    let adjusted = decimal_adjusted(v).unwrap_or(0);
    adjusted >= ctx_int_field(ctx, "Emin", -999999)
        && adjusted <= ctx_int_field(ctx, "Emax", 999999)
}

/// `is_subnormal()`.
pub fn decimal_is_subnormal(ctx: &PyObjectRef, v: &DecValue) -> bool {
    if v.special != DecSpecial::Finite || v.is_zero() {
        return false;
    }
    decimal_adjusted(v).unwrap_or(0) < ctx_int_field(ctx, "Emin", -999999)
}

thread_local! {
    static DECIMAL_CONTEXT_MANAGER_TYPE: std::cell::RefCell<Option<PyObjectRef>> =
        std::cell::RefCell::new(None);
    static DECIMAL_TUPLE_TYPE: std::cell::RefCell<Option<PyObjectRef>> =
        std::cell::RefCell::new(None);
}

/// `localcontext()`'s return value: a context manager that installs the new
/// context on `__enter__` (returning it, so `as ctx` gives a real `Context`)
/// and restores the previous one on `__exit__`.
pub fn make_context_manager(ctx: PyObjectRef) -> PyObjectRef {
    let typ = DECIMAL_CONTEXT_MANAGER_TYPE.with(|c| {
        if let Some(existing) = c.borrow().clone() {
            return existing;
        }
        let mut dict: HashMap<String, PyObjectRef> = HashMap::new();
        dict.insert_str(
            "__enter__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__enter__".to_string(),
                func: |args: &[PyObjectRef]| {
                    let inner = args[0]
                        .borrow()
                        .get_attribute("_ctx")
                        .map_err(|_| PyError::runtime_error("no context"))?;
                    let previous = current_context();
                    DECIMAL_CTX_STACK.with(|s| s.borrow_mut().push(previous));
                    set_current_context(&inner);
                    Ok(inner)
                },
            }),
        );
        dict.insert_str(
            "__exit__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__exit__".to_string(),
                func: |_args: &[PyObjectRef]| {
                    let previous = DECIMAL_CTX_STACK.with(|s| s.borrow_mut().pop());
                    if let Some(ctx) = previous {
                        set_current_context(&ctx);
                    }
                    Ok(py_none())
                },
            }),
        );
        let typ = PyObjectRef::new(PyObject::Type {
            name: "ContextManager".to_string(),
            dict: Box::new(str_map_to_typedict(dict)),
            bases: vec![],
            mro: vec![],
        });
        *c.borrow_mut() = Some(typ.clone());
        typ
    });
    let mut dict = AttrMap::new();
    dict.insert_str("_ctx", ctx);
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

/// `decimal.DecimalTuple` — the named-tuple type `Decimal.as_tuple()`
/// returns (`sign, digits, exponent`), indexable and iterable.
pub fn decimal_tuple_type() -> PyObjectRef {
    DECIMAL_TUPLE_TYPE.with(|c| {
        if let Some(existing) = c.borrow().clone() {
            return existing;
        }
        let mut dict: HashMap<String, PyObjectRef> = HashMap::new();
        dict.insert_str("__module__", py_str("decimal"));
        let typ = PyObjectRef::new(PyObject::Type {
            name: "DecimalTuple".to_string(),
            dict: Box::new(str_map_to_typedict(dict)),
            bases: vec![],
            mro: vec![],
        });
        *c.borrow_mut() = Some(typ.clone());
        typ
    })
}

/// Build a `DecimalTuple` instance: `(sign, digits, exponent)`. `exponent`
/// is a PyObject because the special values carry a string there
/// (`'F'`, `'n'`, `'N'`), exactly like CPython's `as_tuple()`.
pub fn make_decimal_tuple(sign: i64, digits: PyObjectRef, exponent: PyObjectRef) -> PyObjectRef {
    let typ = decimal_tuple_type();
    let mut dict = AttrMap::new();
    dict.insert_str("sign", py_int(sign));
    dict.insert_str("digits", digits);
    dict.insert_str("exponent", exponent);
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

/// `next_plus`/`next_minus`: the representable neighbour one unit in the
/// given direction on the context's full-precision grid. The coefficient is
/// first padded to `prec` digits (which shifts the exponent down), then one
/// is added or subtracted — matching libmpdec, e.g.
/// `Decimal('1.5').next_minus()` is `1.4999...9` (28 digits) rather than
/// `1.49`.
pub fn decimal_next(ctx: &PyObjectRef, v: &DecValue, direction: i64) -> PyResult<DecValue> {
    if v.special == DecSpecial::Infinity {
        return Ok(v.clone());
    }
    if v.special != DecSpecial::Finite {
        return Err(decimal_invalid_op("next"));
    }
    let prec = ctx_prec(ctx) as i64;
    if v.is_zero() {
        let etiny = ctx_int_field(ctx, "Etiny", -1000000);
        return Ok(DecValue {
            special: DecSpecial::Finite,
            sign: direction < 0,
            coeff: BigInt::from(1),
            exp: etiny,
        });
    }
    let digits = digit_count(&v.coeff) as i64;
    let pad = (prec - digits).max(0);
    let padded = &v.coeff * ten_pow(pad);
    let stepped = if direction > 0 {
        &padded + BigInt::from(1)
    } else {
        &padded - BigInt::from(1)
    };
    let mut out = DecValue {
        special: DecSpecial::Finite,
        sign: v.sign,
        coeff: if stepped < BigInt::zero() {
            BigInt::zero()
        } else {
            stepped
        },
        exp: v.exp - pad,
    };
    // Clamp to the context's exponent window (libmpdec signals Clamped).
    let etiny = ctx_int_field(ctx, "Etiny", -1000000);
    if out.exp < etiny {
        out.exp = etiny;
        if digit_count(&out.coeff) > ctx_prec(ctx) {
            out.coeff = ten_pow(ctx_prec(ctx) as i64) - 1;
        }
    }
    Ok(out)
}

/// `next_toward(other)` — the representable neighbour of `self` closest to
/// `other` (the step is toward `other`, and the result keeps `self`'s sign).
pub fn decimal_next_toward(a: &DecValue, other: &DecValue) -> PyResult<DecValue> {
    use std::cmp::Ordering;
    let ordering = decimal_compare(a, other).ok_or_else(|| decimal_invalid_op("next_toward"))?;
    if ordering == Ordering::Equal {
        return Ok(a.clone());
    }
    if a.special != DecSpecial::Finite {
        return Ok(a.clone());
    }
    // Step one ulp of a's own precision grid in the direction of `other`.
    let step = DecValue {
        special: DecSpecial::Finite,
        sign: a.sign,
        coeff: BigInt::from(1),
        exp: a.exp,
    };
    let toward_other = match (a.sign, other.sign, ordering) {
        (false, false, Ordering::Less) | (true, true, Ordering::Greater) => true,
        _ => false,
    };
    let mut out = if toward_other {
        decimal_add(a, &step)?
    } else {
        decimal_sub(a, &step)?
    };
    // Never cross zero.
    if out.sign != a.sign && !out.is_zero() {
        out.sign = a.sign;
    }
    Ok(out)
}
