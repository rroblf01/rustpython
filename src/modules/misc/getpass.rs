use crate::object::*;
use std::collections::HashMap;

pub fn create_getpass_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! getpass_func {
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
    getpass_func!("getuser", |_| {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        Ok(py_str(&user))
    });
    getpass_func!("getpass", |args| {
        let prompt = if args.is_empty() {
            "Password: ".to_string()
        } else {
            args[0].str()
        };
        // In this minimal native implementation, we echo the prompt and read a line from stdin.
        // This is simplified — a real getpass would disable terminal echo.
        print!("{}", prompt);
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut password = String::new();
        match std::io::stdin().read_line(&mut password) {
            Ok(_) => Ok(py_str(password.trim_end())),
            Err(_) => Err(PyError::runtime_error("failed to read password")),
        }
    });
    d
}
