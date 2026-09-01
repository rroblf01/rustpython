use crate::object::*;
use std::collections::HashMap;
// ---- graphlib.TopologicalSorter ----
//
// The graph is stored as a real dict (node -> list of predecessors) under a
// reserved instance-dict key, keyed by genuine PyObjectRef equality/hashing
// (via PyDict) so arbitrary hashable nodes work, not just strings.

thread_local! {
    static TOPOSORTER_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

const TOPOSORTER_GRAPH_KEY: &str = "_graph";
const TOPOSORTER_DONE_KEY: &str = "_done";
const TOPOSORTER_PREPARED_KEY: &str = "_prepared";
const TOPOSORTER_STARTED_KEY: &str = "_started";
const TOPOSORTER_PASSOUT_KEY: &str = "_passedout";

fn toposorter_graph(obj: &PyObjectRef) -> Option<PyObjectRef> {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        dict.get(TOPOSORTER_GRAPH_KEY).cloned()
    } else {
        None
    }
}

/// Read a boolean state flag off the instance's own dict.
fn toposorter_inst_flag(obj: &PyObjectRef, key: &str) -> bool {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        dict.get(key).map(|v| v.truthy()).unwrap_or(false)
    } else {
        false
    }
}

fn toposorter_set_inst_flag(obj: &PyObjectRef, key: &str, val: bool) {
    if let PyObject::Instance { dict, .. } = &mut *obj.borrow_mut() {
        dict.insert(key.to_string(), py_bool(val));
    }
}

/// Keys of one of the flag dicts (`_done`/`_passedout`).
fn toposorter_flag_dict_keys(obj: &PyObjectRef, key: &str) -> Vec<PyObjectRef> {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        if let Some(d) = dict.get(key) {
            let db = d.borrow();
            if let PyObject::Dict(pd) = &*db {
                return pd.keys();
            }
        }
    }
    Vec::new()
}

fn toposorter_set_flag_dict(obj: &PyObjectRef, key: &str, node: PyObjectRef) {
    if let PyObject::Instance { dict, .. } = &mut *obj.borrow_mut() {
        if let Some(d) = dict.get(key) {
            if let PyObject::Dict(pd) = &mut *d.borrow_mut() {
                let _ = pd.set(node, py_bool(true));
            }
        }
    }
}

fn toposorter_flag_dict_has(obj: &PyObjectRef, key: &str, node: &PyObjectRef) -> bool {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        if let Some(d) = dict.get(key) {
            let db = d.borrow();
            if let PyObject::Dict(pd) = &*db {
                return pd.get(node).ok().flatten().is_some();
            }
        }
    }
    false
}

fn toposorter_done_items(obj: &PyObjectRef) -> Vec<PyObjectRef> {
    toposorter_flag_dict_keys(obj, TOPOSORTER_DONE_KEY)
}

fn toposorter_ensure_node(graph: &PyObjectRef, node: &PyObjectRef) -> PyResult<()> {
    let mut g = graph.borrow_mut();
    if let PyObject::Dict(d) = &mut *g {
        if d.get(node)?.is_none() {
            d.set(node.clone(), py_list(vec![]))?;
        }
    }
    Ok(())
}

fn toposorter_add_edge(
    graph: &PyObjectRef,
    node: &PyObjectRef,
    pred: &PyObjectRef,
) -> PyResult<()> {
    // Ensure the NODE before the pred so the graph dict's insertion order
    // matches real CPython's `_node2info` (the node first) — cycle reporting
    // starts from the first node in that order, and the test asserts the
    // exact cycle node sequence.
    toposorter_ensure_node(graph, node)?;
    toposorter_ensure_node(graph, pred)?;
    let mut g = graph.borrow_mut();
    if let PyObject::Dict(d) = &mut *g {
        match d.get(node)? {
            Some(preds_ref) => {
                if let PyObject::List(items) = &mut *preds_ref.borrow_mut() {
                    items.push(pred.clone());
                }
            }
            None => {
                d.set(node.clone(), py_list(vec![pred.clone()]))?;
            }
        }
    }
    Ok(())
}

/// Find one cycle, replicating CPython `graphlib._find_cycle`: an iterative
/// DFS that follows SUCCESSOR edges (the nodes that DEPEND on each node),
/// iterating `_node2info` in insertion order and successors in CPython's
/// small-int set iteration order (ascending). Returns `stack[first:] + [node]`
/// — the repeated start node is INCLUDED (real `CycleError.args[1]`).
fn toposorter_find_cycle(graph: &PyObjectRef, _leftover: &[PyObjectRef]) -> Vec<PyObjectRef> {
    let entries = {
        let g = graph.borrow();
        match &*g {
            PyObject::Dict(d) => d.items(),
            _ => Vec::new(),
        }
    };
    // node order = graph dict insertion order (matches CPython's _node2info)
    let nodes: Vec<PyObjectRef> = entries.iter().map(|(n, _)| n.clone()).collect();
    // preds[node] = the nodes node depends on
    let preds_of = |node: &PyObjectRef| -> Vec<PyObjectRef> {
        if let Ok(Some(p)) = {
            let g = graph.borrow();
            if let PyObject::Dict(d) = &*g {
                d.get(node)
            } else {
                Ok(None)
            }
        } {
            let pb = p.borrow();
            if let PyObject::List(items) = &*pb {
                return items.clone();
            }
        }
        Vec::new()
    };
    // successors[node] = nodes that depend on node (reverse edges)
    let successors_of = |node: &PyObjectRef| -> Vec<PyObjectRef> {
        let mut succs = Vec::new();
        for n in &nodes {
            if preds_of(n).iter().any(|p| p.equals(node).unwrap_or(false)) {
                succs.push(n.clone());
            }
        }
        // CPython's set iteration for small ints is ascending — match it.
        if succs.iter().all(|s| s.as_i64().is_some()) {
            succs.sort_by_key(|s| s.as_i64().unwrap());
        }
        succs
    };

    let mut seen: Vec<PyObjectRef> = Vec::new();
    for start in &nodes {
        if seen.iter().any(|s| s.equals(start).unwrap_or(false)) {
            continue;
        }
        let mut stack: Vec<PyObjectRef> = Vec::new();
        let mut node2stacki: Vec<PyObjectRef> = Vec::new(); // in stack order
        let mut node = start.clone();
        loop {
            if seen.iter().any(|s| s.equals(&node).unwrap_or(false)) {
                if let Some(pos) = node2stacki
                    .iter()
                    .position(|n| n.equals(&node).unwrap_or(false))
                {
                    let mut cycle = stack[pos..].to_vec();
                    cycle.push(node.clone());
                    return cycle;
                }
            } else {
                seen.push(node.clone());
                node2stacki.push(node.clone());
                stack.push(node.clone());
            }
            // backtrack to topmost stack entry with another successor
            let mut descended = false;
            while !stack.is_empty() {
                let top = stack.last().unwrap().clone();
                let succs = successors_of(&top);
                // find the next successor NOT yet fully processed
                let next_succ = succs
                    .iter()
                    .find(|s| {
                        // if already seen and not in current stack, skip (state 2)
                        let already = seen.iter().any(|x| x.equals(*s).unwrap_or(false));
                        let in_stack = node2stacki.iter().any(|x| x.equals(*s).unwrap_or(false));
                        !(already && !in_stack)
                    })
                    .cloned();
                match next_succ {
                    Some(s) => {
                        node = s;
                        descended = true;
                        break;
                    }
                    None => {
                        stack.pop();
                        node2stacki.pop();
                    }
                }
            }
            if !descended {
                break;
            }
        }
    }
    Vec::new()
}

/// Kahn's algorithm over the stored graph. Returns the sorted node list, or
/// an error (CycleError) if the graph isn't a DAG.
fn toposorter_sorted_order(graph: &PyObjectRef) -> PyResult<Vec<PyObjectRef>> {
    let entries = {
        let g = graph.borrow();
        match &*g {
            PyObject::Dict(d) => d.items(),
            _ => return Err(PyError::runtime_error("corrupt TopologicalSorter graph")),
        }
    };
    let mut remaining: Vec<(PyObjectRef, Vec<PyObjectRef>)> = Vec::with_capacity(entries.len());
    for (node, preds_ref) in &entries {
        let preds = match &*preds_ref.borrow() {
            PyObject::List(items) => items.clone(),
            _ => vec![],
        };
        remaining.push((node.clone(), preds));
    }

    let mut result: Vec<PyObjectRef> = Vec::with_capacity(remaining.len());
    loop {
        let mut ready = Vec::new();
        let mut still_pending = Vec::new();
        for (node, preds) in remaining {
            let all_ready = preds
                .iter()
                .all(|p| result.iter().any(|r| r.equals(p).unwrap_or(false)));
            if all_ready {
                ready.push(node);
            } else {
                still_pending.push((node, preds));
            }
        }
        if ready.is_empty() {
            remaining = still_pending;
            break;
        }
        result.extend(ready);
        remaining = still_pending;
        if remaining.is_empty() {
            break;
        }
    }

    if !remaining.is_empty() {
        let leftover: Vec<PyObjectRef> = remaining.into_iter().map(|(n, _)| n).collect();
        let cycle = toposorter_find_cycle(graph, &leftover);
        return Err(PyError::Exception(
            "CycleError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "CycleError".to_string(),
                args: vec![py_str("nodes are in a cycle"), py_list(cycle)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        ));
    }
    Ok(result)
}

fn build_topological_sorter_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }

    type_dict.insert_str(
        "__init__",
        bf!("__init__", |args| {
            let graph = py_dict();
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert(TOPOSORTER_GRAPH_KEY.to_string(), graph.clone());
                dict.insert(TOPOSORTER_DONE_KEY.to_string(), py_dict());
                dict.insert(TOPOSORTER_PASSOUT_KEY.to_string(), py_dict());
            }
            // Optional initial graph: {node: iterable_of_predecessors, ...}
            if args.len() > 1 {
                let entries = match &*args[1].borrow() {
                    PyObject::Dict(d) => d.items(),
                    PyObject::None => vec![],
                    _ => return Err(PyError::type_error("graph argument must be a dict")),
                };
                for (node, preds) in entries {
                    toposorter_ensure_node(&graph, &node)?;
                    // Preds may be ANY iterable (list, tuple, set, a generator,
                    // an EMPTY DICT literal `{}` — which is not a set — etc.).
                    // Treating the value as a single predecessor (the previous
                    // `_ => vec![preds.clone()]` fallback) broke `{1: {}}`
                    // (an empty dict → hashing a dict → "unhashable type").
                    let it = builtin_iter(&[preds])?;
                    loop {
                        match builtin_next(&[it.clone()]) {
                            Ok(p) => toposorter_add_edge(&graph, &node, &p)?,
                            Err(PyError::StopIteration) => break,
                            Err(e) => return Err(e),
                        }
                    }
                }
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "add",
        bf!("add", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error(
                    "add() missing required argument: 'node'",
                ));
            }
            let graph = toposorter_graph(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a TopologicalSorter"))?;
            let node = &args[1];
            if args.len() > 2 {
                for pred in &args[2..] {
                    toposorter_add_edge(&graph, node, pred)?;
                }
            } else {
                toposorter_ensure_node(&graph, node)?;
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "prepare",
        bf!("prepare", |args| {
            let graph = toposorter_graph(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a TopologicalSorter"))?;
            // Real graphlib: prepare() may be called repeatedly BEFORE get_ready()
            // (test_prepare_multiple_times), but NOT once the sort has started.
            let started = toposorter_inst_flag(&args[0], TOPOSORTER_STARTED_KEY);
            if started {
                return Err(PyError::value_error("cannot prepare() after starting sort"));
            }
            toposorter_set_inst_flag(&args[0], TOPOSORTER_PREPARED_KEY, true);
            // Validates the graph is acyclic up front, matching real prepare().
            toposorter_sorted_order(&graph)?;
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "static_order",
        bf!("static_order", |args| {
            let graph = toposorter_graph(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a TopologicalSorter"))?;
            let order = toposorter_sorted_order(&graph)?;
            Ok(py_list(order))
        }),
    );
    type_dict.insert_str(
        "get_ready",
        bf!("get_ready", |args| {
            if !toposorter_inst_flag(&args[0], TOPOSORTER_PREPARED_KEY) {
                return Err(PyError::value_error("prepare() must be called first"));
            }
            toposorter_set_inst_flag(&args[0], TOPOSORTER_STARTED_KEY, true);
            let graph = toposorter_graph(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a TopologicalSorter"))?;
            let done_items: Vec<PyObjectRef> = toposorter_done_items(&args[0]);
            let passedout_items: Vec<PyObjectRef> =
                toposorter_flag_dict_keys(&args[0], TOPOSORTER_PASSOUT_KEY);
            let entries = match &*graph.borrow() {
                PyObject::Dict(d) => d.items(),
                _ => vec![],
            };
            let mut ready = Vec::new();
            for (node, preds_ref) in entries {
                if done_items.iter().any(|d| d.equals(&node).unwrap_or(false)) {
                    continue;
                }
                if passedout_items
                    .iter()
                    .any(|d| d.equals(&node).unwrap_or(false))
                {
                    continue;
                }
                let preds = match &*preds_ref.borrow() {
                    PyObject::List(v) => v.clone(),
                    _ => vec![],
                };
                let all_done = preds
                    .iter()
                    .all(|p| done_items.iter().any(|d| d.equals(p).unwrap_or(false)));
                if all_done {
                    ready.push(node);
                }
            }
            // Mark the returned nodes as passed out (a second get_ready() call
            // returns nothing until done() is called, matching real graphlib).
            for node in &ready {
                toposorter_set_flag_dict(&args[0], TOPOSORTER_PASSOUT_KEY, node.clone());
            }
            Ok(py_tuple(ready))
        }),
    );
    type_dict.insert_str(
        "done",
        bf!("done", |args| {
            if !toposorter_inst_flag(&args[0], TOPOSORTER_PREPARED_KEY) {
                return Err(PyError::value_error("prepare() must be called first"));
            }
            let graph = toposorter_graph(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a TopologicalSorter"))?;
            for node in &args[1..] {
                // node must have been added via add()/the graph
                let exists = match &*graph.borrow() {
                    PyObject::Dict(d) => d.get(node).ok().flatten().is_some(),
                    _ => false,
                };
                if !exists {
                    return Err(PyError::value_error(format!(
                        "node {} was not added using add()",
                        node.repr()
                    )));
                }
                // node must have been passed out by get_ready()
                if !toposorter_flag_dict_has(&args[0], TOPOSORTER_PASSOUT_KEY, node) {
                    return Err(PyError::value_error(format!(
                        "node {} was not passed out",
                        node.repr()
                    )));
                }
                toposorter_set_flag_dict(&args[0], TOPOSORTER_DONE_KEY, node.clone());
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "is_active",
        bf!("is_active", |args| {
            if !toposorter_inst_flag(&args[0], TOPOSORTER_PREPARED_KEY) {
                return Err(PyError::value_error("prepare() must be called first"));
            }
            let graph = toposorter_graph(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a TopologicalSorter"))?;
            let total = match &*graph.borrow() {
                PyObject::Dict(d) => d.len(),
                _ => 0,
            };
            let done_count = toposorter_done_items(&args[0]).len();
            Ok(py_bool(done_count < total))
        }),
    );
    type_dict.insert_str(
        "__bool__",
        bf!("__bool__", |args| {
            let graph = toposorter_graph(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a TopologicalSorter"))?;
            let non_empty = match &*graph.borrow() {
                PyObject::Dict(d) => !d.is_empty(),
                _ => false,
            };
            Ok(py_bool(non_empty))
        }),
    );

    PyObjectRef::new(PyObject::Type {
        name: "TopologicalSorter".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

fn get_topological_sorter_type() -> PyObjectRef {
    let existing = TOPOSORTER_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_topological_sorter_type();
    TOPOSORTER_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

pub fn create_graphlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("TopologicalSorter", get_topological_sorter_type());
    d.insert_str(
        "CycleError",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "CycleError".to_string(),
            func: crate::object::builtin_make_exception_cycleerror,
        }),
    );
    d
}
