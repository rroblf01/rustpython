use crate::object::*;
use std::collections::HashMap;

pub fn create_queue_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! q_func {
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

    q_func!("Queue", |_args| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(QueueInner {
            queue: std::collections::VecDeque::new(),
        }));
        Ok(PyObjectRef::new(PyObject::Queue(inner)))
    });

    d
}
