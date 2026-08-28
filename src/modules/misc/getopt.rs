use crate::object::*;
use std::collections::HashMap;

pub fn create_getopt_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! getopt_func {
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

    // Helper: check if a short option expects an argument (followed by ':' in shortopts)
    fn short_has_arg(c: char, shortopts: &str) -> bool {
        if let Some(pos) = shortopts.find(c) {
            shortopts.as_bytes().get(pos + 1) == Some(&b':')
        } else {
            false
        }
    }

    getopt_func!("getopt", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "getopt() requires at least 2 arguments (args, shortopts)",
            ));
        }
        let shortopts = args[1].str();
        // Parse longopts if provided (third argument is a list of long option names)
        let longopts: Vec<String> = if args.len() > 2 {
            if let PyObject::List(list) = &*args[2].borrow() {
                list.iter().map(|s| s.str()).collect()
            } else {
                return Err(PyError::type_error("longopts must be a list"));
            }
        } else {
            Vec::new()
        };

        // Extract the argument list from the first argument (should be a list of strings)
        let arg_list: Vec<String> = if let PyObject::List(list) = &*args[0].borrow() {
            list.iter().map(|s| s.str()).collect()
        } else {
            return Err(PyError::type_error("args must be a list"));
        };

        let mut opts: Vec<PyObjectRef> = Vec::new();
        let mut positional: Vec<PyObjectRef> = Vec::new();
        // Process EVERY arg from index 0 — the caller decides whether to pass
        // sys.argv (program name included) or sys.argv[1:] (options only).
        // The previous `i = 1` skip silently dropped a leading option
        // (real trigger: quopri.main's `getopt.getopt(sys.argv[1:], 'td')`
        // with sys.argv[1:] == ['-d'] — the '-d' was skipped, so decode was
        // never enabled).
        let mut i: usize = 0;
        let mut options_done = false;

        while i < arg_list.len() {
            let arg = &arg_list[i];
            if options_done || !arg.starts_with('-') {
                positional.push(py_str(arg));
                i += 1;
                if arg.starts_with('-') {
                    options_done = true;
                }
                continue;
            }
            if arg == "--" {
                options_done = true;
                i += 1;
                continue;
            }
            if arg.starts_with("--") {
                // Long option
                let opt_name = &arg[2..];
                let (name, val) = if let Some(eq_pos) = opt_name.find('=') {
                    (&opt_name[..eq_pos], Some(&opt_name[eq_pos + 1..]))
                } else {
                    (opt_name, None)
                };
                // Check if this long option expects an argument
                let needs_val = longopts.iter().any(|lo| {
                    let base = if lo.ends_with('=') {
                        &lo[..lo.len() - 1]
                    } else {
                        lo.as_str()
                    };
                    base == name && lo.ends_with('=')
                });
                match val {
                    Some(v) => opts.push(py_tuple(vec![py_str(&format!("--{}", name)), py_str(v)])),
                    None => {
                        if needs_val {
                            i += 1;
                            if i < arg_list.len() {
                                opts.push(py_tuple(vec![
                                    py_str(&format!("--{}", name)),
                                    py_str(&arg_list[i]),
                                ]));
                            } else {
                                return Err(PyError::type_error(&format!(
                                    "option --{} requires a value",
                                    name
                                )));
                            }
                        } else {
                            opts.push(py_tuple(vec![py_str(&format!("--{}", name)), py_str("")]));
                        }
                    }
                }
                i += 1;
            } else {
                // Short option(s)
                let chars: Vec<char> = arg[1..].chars().collect();
                for (j, c) in chars.iter().enumerate() {
                    if !shortopts.contains(*c) {
                        return Err(PyError::type_error(&format!(
                            "option -{} not recognized",
                            c
                        )));
                    }
                    if short_has_arg(*c, &shortopts) {
                        if j + 1 < chars.len() {
                            // Value attached: -xvalue
                            let val: String = chars[j + 1..].iter().collect();
                            opts.push(py_tuple(vec![py_str(&format!("-{}", c)), py_str(&val)]));
                            break;
                        } else {
                            i += 1;
                            if i < arg_list.len() {
                                opts.push(py_tuple(vec![
                                    py_str(&format!("-{}", c)),
                                    py_str(&arg_list[i]),
                                ]));
                            } else {
                                return Err(PyError::type_error(&format!(
                                    "option -{} requires an argument",
                                    c
                                )));
                            }
                        }
                    } else {
                        opts.push(py_tuple(vec![py_str(&format!("-{}", c)), py_str("")]));
                    }
                }
                i += 1;
            }
        }

        Ok(py_tuple(vec![py_list(opts), py_list(positional)]))
    });
    d
}
