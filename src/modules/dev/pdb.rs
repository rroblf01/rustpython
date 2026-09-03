use crate::object::*;
use std::collections::HashMap;

pub fn create_pdb_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pdb_func {
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

    pdb_func!("set_trace", |_| {
        println!("Debugger not available");
        Ok(py_none())
    });
    // pdb.Pdb stub needed by test_pdb / test_remote_pdb / test_pyclbr
    {
        let mut pdb_type_dict = HashMap::new();
        pdb_type_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction{ name:"__init__".to_string(), func:|args|{
            if let Some(inst)=args.first(){ if let PyObject::Instance{dict,..}= &mut *inst.borrow_mut(){ dict.insert("curframe".to_string(), py_none()); }}
            Ok(py_none())
        }}));
        let pdb_type = PyObjectRef::new(PyObject::Type{ name:"Pdb".to_string(), dict: Box::new(str_map_to_typedict(pdb_type_dict)), bases:vec![], mro:vec![]});
        d.insert_str("Pdb", pdb_type.clone());
        d.insert_str("_PdbServer", pdb_type);
    }

    d
}
