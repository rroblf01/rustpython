use crate::object::*;
use std::collections::HashMap;
use std::rc::Rc;

pub fn create_sqlite3_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    d.insert("connect".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "connect".to_string(),
        func: sqlite3_connect,
    }));
    d.insert("OperationalError".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "OperationalError".to_string(),
        func: |args| {
            let msg = if args.is_empty() { "".to_string() } else { args[0].str() };
            Ok(PyObjectRef::new(PyObject::Exception {
                typ: "OperationalError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
            }))
        },
    }));
    d
}

fn sqlite3_connect(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("connect() requires database path"));
    }
    let database = args[0].str();
    let conn = rusqlite::Connection::open(&database)
        .map_err(|e| PyError::runtime_error(format!("sqlite3: {}", e)))?;
    let conn = Rc::new(std::cell::RefCell::new(conn));
    Ok(create_connection(conn))
}

fn create_connection(conn: Rc<std::cell::RefCell<rusqlite::Connection>>) -> PyObjectRef {
    let conn_cursor = conn.clone();
    let conn_exec = conn.clone();

    // Use thread_local to pass the connection reference to cursor/execute functions
    thread_local! {
        static CONN_CURSOR: std::cell::RefCell<Option<Rc<std::cell::RefCell<rusqlite::Connection>>>> = std::cell::RefCell::new(None);
        static CONN_EXEC: std::cell::RefCell<Option<Rc<std::cell::RefCell<rusqlite::Connection>>>> = std::cell::RefCell::new(None);
    }
    CONN_CURSOR.with(|c| *c.borrow_mut() = Some(conn_cursor));
    CONN_EXEC.with(|c| *c.borrow_mut() = Some(conn_exec));

    PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Type {
            name: "sqlite3.Connection".to_string(),
            dict: HashMap::from([
                ("cursor".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "cursor".to_string(),
                    func: |_args| {
                        CONN_CURSOR.with(|c| {
                            let _conn = c.borrow().clone();
                        });
                        Ok(PyObjectRef::new(PyObject::Instance {
                            typ: PyObjectRef::new(PyObject::Type {
                                name: "sqlite3.Cursor".to_string(),
                                dict: HashMap::from([
                                    ("__iter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                                        name: "__iter__".to_string(),
                                        func: |args| Ok(args[0].clone()),
                                    })),
                                    ("__next__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                                        name: "__next__".to_string(),
                                        func: |_args| Err(PyError::StopIteration),
                                    })),
                                ]),
                                bases: vec![],
                                mro: vec![],
                            }),
                            dict: HashMap::new(),
                        }))
                    },
                })),
                ("execute".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "execute".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("execute() requires sql"));
                        }
                        let sql = args[1].str();
                        CONN_EXEC.with(|c| {
                            let conn = c.borrow();
                            if let Some(ref conn) = *conn {
                                let conn = conn.borrow();
                                conn.execute(&sql, []).map_err(|e| PyError::runtime_error(format!("sqlite3: {}", e)))?;
                            }
                            Ok::<_, PyError>(())
                        })?;
                        Ok(py_none())
                    },
                })),
                ("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "close".to_string(),
                    func: |_args| Ok(py_none()),
                })),
            ]),
            bases: vec![],
            mro: vec![],
        }),
        dict: HashMap::new(),
    })
}
