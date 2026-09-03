use crate::object::*;
use std::collections::HashMap;

pub fn create_traceback_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tb_func {
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

    tb_func!("format_exc", |_| { Ok(py_str("")) });

    tb_func!("print_exc", |_| {
        println!("No traceback");
        Ok(py_none())
    });
    tb_func!("extract_stack", |_| { Ok(py_list(vec![])) });
    tb_func!("extract_tb", |_| { Ok(py_list(vec![])) });
    tb_func!("format_stack", |_| { Ok(py_list(vec![])) });
    tb_func!("format_tb", |_| { Ok(py_list(vec![])) });
    tb_func!("print_stack", |_| Ok(py_none()));
    tb_func!("clear_frames", |_| Ok(py_none()));
    // traceback.TracebackException stub
    {
        let mut te_dict = HashMap::new();
        te_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction{ name:"__init__".to_string(), func:|args|{
            if args.is_empty(){ return Ok(py_none()); }
            // store exc_type etc into instance dict if Instance
            if let PyObject::Instance{dict,..}= &mut *args[0].borrow_mut(){ dict.insert("exc_type".to_string(), args.get(1).cloned().unwrap_or_else(py_none)); }
            Ok(py_none())
        }}));
        let te_type = PyObjectRef::new(PyObject::Type{ name:"TracebackException".to_string(), dict: Box::new(str_map_to_typedict(te_dict)), bases:vec![], mro:vec![]});
        d.insert_str("TracebackException", te_type);
    }

    d
}
