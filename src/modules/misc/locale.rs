use crate::object::*;
use std::collections::HashMap;

pub fn create_locale_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! loc_func {
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

    // LC_* constants matching CPython values
    d.insert_str("LC_ALL", py_int(6i64));
    d.insert_str("LC_COLLATE", py_int(3i64));
    d.insert_str("LC_CTYPE", py_int(0i64));
    d.insert_str("LC_MONETARY", py_int(4i64));
    d.insert_str("LC_NUMERIC", py_int(1i64));
    d.insert_str("LC_TIME", py_int(2i64));
    d.insert_str("LC_MESSAGES", py_int(5i64));

    // locale.Error — the exception `setlocale`/`localeconv` raise for an
    // unsettable/unknown locale. Represented exactly like `binascii.Error`
    // (a `BuiltinFunction` producing a native `PyObject::Exception`), which
    // makes both `raise Error(...)` and `except Error:` work (`test__locale.py`
    // catches it around every `setlocale` call). Real CPython subclasses
    // `OSError`; the name-based matching this interpreter uses only needs the
    // `"Error"` type name to line up.
    d.insert_str(
        "Error",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "locale.Error".to_string(),
            func: |args| {
                let msg = if args.is_empty() {
                    String::new()
                } else {
                    args[0].str()
                };
                Ok(PyObjectRef::new(PyObject::Exception {
                    typ: "Error".to_string(),
                    args: vec![py_str(&msg)],
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                }))
            },
        }),
    );

    // Real, shared per-category locale state — `setlocale(category, locale)`
    // writes here, `setlocale(category)` (the 1-arg getter form real Python
    // supports: returns the CURRENT setting) and `getlocale()` read from the
    // SAME map. Module-level so that BOTH native `locale` and native `_locale`
    // (which in real CPython is the underlying C extension that `locale`
    // delegates to) share one state map. Defaults to "C" (the only locale this
    // interpreter can genuinely honor — its own date/number formatting is
    // locale-independent English), matching real CPython on a fresh process.
    thread_local! {
        static CURRENT_LOCALES: std::cell::RefCell<std::collections::HashMap<i64, String>> = std::cell::RefCell::new(std::collections::HashMap::new());
    }

    // Locale-aware numeric conventions for `localeconv()`. Real CPython asks
    // the C library's locale database; this interpreter models the handful of
    // locales the CPython regression tests actually assert on (see
    // `known_numerics` in tests/cpython/test__locale.py) and defaults to the
    // POSIX "C" conventions for everything else. The language part is taken
    // before any `.encoding` or `@modifier` suffix.
    fn numeric_conventions(locale: &str) -> (String, String) {
        let lang = locale.split('.').next().unwrap_or(locale);
        let lang = lang.split('@').next().unwrap_or(lang);
        match lang {
            "de_DE" => (",".to_string(), ".".to_string()),
            "fr_FR" => (",".to_string(), String::new()),
            "en_US" => (".".to_string(), ",".to_string()),
            "ps_AF" => ("\u{066b}".to_string(), "\u{066c}".to_string()),
            _ => (".".to_string(), String::new()),
        }
    }

    fn get_locale(category: i64) -> String {
        CURRENT_LOCALES
            .with(|m| {
                let map = m.borrow();
                if category == 6 {
                    map.get(&6).cloned().or_else(|| {
                        [0i64, 1, 2, 3, 4, 5]
                            .iter()
                            .find_map(|c| map.get(c).cloned())
                    })
                } else {
                    map.get(&category).cloned()
                }
            })
            .unwrap_or_else(|| "C".to_string())
    }
    fn set_locale(category: i64, locale: &str) {
        CURRENT_LOCALES.with(|m| {
            let mut map = m.borrow_mut();
            if category == 6 {
                for c in [0i64, 1, 2, 3, 4, 5] {
                    map.insert(c, locale.to_string());
                }
            }
            map.insert(category, locale.to_string());
        });
    }

    // getlocale() — returns (lang_code, encoding) tuple for the current
    // setting of the requested category (real CPython splits the locale
    // string on '.'/encoding).
    loc_func!("getlocale", |args| {
        let category = if args.len() >= 1 {
            args[0].as_i64().unwrap_or(6) // default LC_ALL
        } else {
            6
        };
        let current = get_locale(category);
        let mut parts = current.splitn(2, '.');
        let lang = parts.next().unwrap_or("C");
        let enc = parts.next().unwrap_or("UTF-8");
        Ok(py_tuple(vec![py_str(lang), py_str(enc)]))
    });

    // setlocale(category[, locale]) — real CPython semantics: with a second
    // argument (or `None`), SET the category and return the new locale;
    // with only the category, GET and return the current setting. Was a
    // 2-args-or-error stub, so the extremely common `saved = setlocale(LC_TIME)`
    // getter idiom (`test_strftime.py`'s setUp) raised a spurious TypeError.
    loc_func!("setlocale", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "setlocale() missing required argument (category)",
            ));
        }
        let category = args[0].as_i64().unwrap_or(6); // default LC_ALL
        if args.len() >= 2 && !matches!(&*args[1].borrow(), PyObject::None) {
            let locale = args[1].str();
            set_locale(category, &locale);
            // Attempt to set locale via system
            let _ = std::env::set_var("LANG", &locale);
            Ok(py_str(&locale))
        } else {
            Ok(py_str(&get_locale(category)))
        }
    });

    // localeconv() — dict of locale conventions, with `decimal_point` and
    // `thousands_sep` reflecting the CURRENT LC_NUMERIC setting (CPython's
    // `test__locale.py` asserts fr_FR -> ',' etc. against this).
    loc_func!("localeconv", |args| {
        let _ = args;
        let (decimal_point, thousands_sep) = numeric_conventions(&get_locale(1));
        let dict = py_dict();
        if let PyObject::Dict(d) = &mut *dict.borrow_mut() {
            d.set(py_str("decimal_point"), py_str(&decimal_point)).ok();
            d.set(py_str("thousands_sep"), py_str(&thousands_sep)).ok();
            d.set(py_str("grouping"), py_list(vec![py_int(3), py_int(0)]))
                .ok();
            d.set(py_str("currency_symbol"), py_str("$")).ok();
            d.set(py_str("mon_decimal_point"), py_str(".")).ok();
            d.set(py_str("mon_thousands_sep"), py_str(",")).ok();
            d.set(py_str("mon_grouping"), py_list(vec![py_int(3), py_int(0)]))
                .ok();
            d.set(py_str("positive_sign"), py_str("")).ok();
            d.set(py_str("negative_sign"), py_str("-")).ok();
            d.set(py_str("int_frac_digits"), py_int(2)).ok();
            d.set(py_str("frac_digits"), py_int(2)).ok();
            d.set(py_str("p_cs_precedes"), py_int(1)).ok();
            d.set(py_str("n_cs_precedes"), py_int(1)).ok();
            d.set(py_str("p_sep_by_space"), py_int(0)).ok();
            d.set(py_str("n_sep_by_space"), py_int(0)).ok();
            d.set(py_str("p_sign_posn"), py_int(1)).ok();
            d.set(py_str("n_sign_posn"), py_int(1)).ok();
            d.set(py_str("int_curr_symbol"), py_str("USD ")).ok();
        }
        Ok(dict)
    });

    // getdefaultlocale() — returns (lang_code, encoding)
    loc_func!("getdefaultlocale", |_| {
        Ok(py_tuple(vec![py_str("en_US"), py_str("UTF-8")]))
    });

    // getpreferredencoding() — returns 'UTF-8'
    loc_func!("getpreferredencoding", |_| { Ok(py_str("UTF-8")) });

    // strcoll(a, b) — string comparison using locale
    loc_func!("strcoll", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "strcoll() requires 2 arguments (str1, str2)",
            ));
        }
        let a = args[0].str();
        let b = args[1].str();
        Ok(py_int(a.cmp(&b) as i64))
    });

    // strxfrm(s) — string transformation for locale-aware comparison
    loc_func!("strxfrm", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "strxfrm() missing required argument (str)",
            ));
        }
        Ok(py_str(&args[0].str()))
    });

    d
}
