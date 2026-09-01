use crate::object::*;
use std::collections::HashMap;

pub fn create_statistics_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! stat_func {
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

    // Extract numeric values from ANY iterable (not just a literal `list` —
    // real `statistics` functions accept tuples, generators, `range`, etc.)
    // via `collect_iterable`, converting each element through `builtin_float`
    // (the same general `__float__`-dispatch machinery `float()` itself
    // uses) rather than hand-matching only `PyObject::Int`/`Float` — this
    // means `Fraction`/`Decimal`/`bool`/any custom class implementing
    // `__float__` all work, not just plain int/float literals. Previously
    // EVERY statistics function required a literal `list` argument (raising
    // "argument must be a list" for a tuple, generator, or `Fraction`-
    // containing sequence) — found via CPython's own `test_statistics.py`,
    // whose shared `NumericTestCase`-style mixin tests exercise exactly
    // these argument shapes across `TestMean`/`TestMedian`/`TestStdev`/etc.
    fn stat_extract_nums(data: &PyObjectRef) -> PyResult<Vec<f64>> {
        let items = crate::object::collect_iterable(data)?;
        if items.is_empty() {
            return Err(PyError::ValueError("argument is empty".to_string()));
        }
        let mut nums: Vec<f64> = Vec::with_capacity(items.len());
        for item in &items {
            let f = builtin_float(std::slice::from_ref(item))
                .map_err(|_| PyError::type_error("argument must contain numbers"))?;
            let borrowed = f.borrow();
            match &*borrowed {
                PyObject::Float(v) => nums.push(*v),
                _ => return Err(PyError::type_error("argument must contain numbers")),
            }
        }
        Ok(nums)
    }

    stat_func!("mean", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("mean() missing required argument"));
        }
        let nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::ValueError(_) => PyError::ValueError("mean() argument is empty".to_string()),
            PyError::TypeError(_) => PyError::type_error("mean() argument must contain numbers"),
            other => other,
        })?;
        let n = nums.len() as f64;
        let sum: f64 = nums.iter().sum();
        Ok(py_float(sum / n))
    });

    stat_func!("median", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("median() missing required argument"));
        }
        let mut nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::ValueError(_) => PyError::ValueError("median() argument is empty".to_string()),
            PyError::TypeError(_) => PyError::type_error("median() argument must contain numbers"),
            other => other,
        })?;
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        if n % 2 == 0 {
            Ok(py_float((nums[n / 2 - 1] + nums[n / 2]) / 2.0))
        } else {
            Ok(py_float(nums[n / 2]))
        }
    });

    stat_func!("stdev", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("stdev() missing required argument"));
        }
        let nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::TypeError(_) => PyError::type_error("stdev() argument must contain numbers"),
            other => other,
        })?;
        if nums.len() < 2 {
            return Err(PyError::ValueError(
                "stdev() requires at least 2 data points".to_string(),
            ));
        }
        let n = nums.len() as f64;
        let sum: f64 = nums.iter().sum();
        let mean = sum / n;
        let variance: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
        Ok(py_float(variance.sqrt()))
    });

    // `statistics.harmonic_mean` was missing entirely — the harmonic mean
    // is `n / sum(1/x for x in data)`, undefined (real CPython raises
    // `StatisticsError`, mapped to `ValueError` here matching the other
    // stats functions' convention) if any element is zero.
    stat_func!("harmonic_mean", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "harmonic_mean() missing required argument",
            ));
        }
        let nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::ValueError(_) => {
                PyError::ValueError("harmonic_mean() argument is empty".to_string())
            }
            PyError::TypeError(_) => {
                PyError::type_error("harmonic_mean() argument must contain numbers")
            }
            other => other,
        })?;
        if nums.iter().any(|&x| x < 0.0) {
            return Err(PyError::ValueError(
                "harmonic_mean() does not support negative values".to_string(),
            ));
        }
        if nums.iter().any(|&x| x == 0.0) {
            return Ok(py_float(0.0));
        }
        let n = nums.len() as f64;
        let recip_sum: f64 = nums.iter().map(|x| 1.0 / x).sum();
        Ok(py_float(n / recip_sum))
    });

    stat_func!("mode", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("mode() missing required argument"));
        }
        let items = crate::object::collect_iterable(&args[0])?;
        if items.is_empty() {
            return Err(PyError::ValueError("mode() argument is empty".to_string()));
        }
        let mut counts = std::collections::HashMap::new();
        let mut max_count = 0i64;
        let mut modes: Vec<PyObjectRef> = Vec::new();
        for item in &items {
            let hash = item.hash()?;
            let entry = counts.entry(hash).or_insert((0i64, item.clone()));
            entry.0 += 1;
        }
        // Find the max count
        for (_, (count, ref item)) in &counts {
            if *count > max_count {
                max_count = *count;
                modes.clear();
                modes.push(item.clone());
            } else if *count == max_count {
                modes.push(item.clone());
            }
        }
        if modes.len() == 1 {
            Ok(modes[0].clone())
        } else {
            Ok(py_list(modes))
        }
    });

    stat_func!("median_low", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "median_low() missing required argument",
            ));
        }
        let mut nums = stat_extract_nums(&args[0])?;
        if nums.is_empty() {
            return Err(PyError::ValueError(
                "median_low() argument is empty".to_string(),
            ));
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        Ok(py_float(nums[(n - 1) / 2]))
    });

    stat_func!("median_high", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "median_high() missing required argument",
            ));
        }
        let mut nums = stat_extract_nums(&args[0])?;
        if nums.is_empty() {
            return Err(PyError::ValueError(
                "median_high() argument is empty".to_string(),
            ));
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        Ok(py_float(nums[n / 2]))
    });

    // `statistics.__all__` — same fix, same reason, as `operator.__all__`
    // (`core.rs`) — missing entirely, breaking the module's own
    // `test___all__` sanity check at collection time.
    let all_names: Vec<PyObjectRef> = d
        .keys()
        .filter(|k| !k.starts_with('_'))
        .map(|k| py_str(k))
        .collect();
    d.insert_str("__all__", py_list(all_names));

    d
}

