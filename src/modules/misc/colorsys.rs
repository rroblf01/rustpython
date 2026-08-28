use crate::object::*;
use std::collections::HashMap;

pub fn create_colorsys_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cs_func {
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

    // Helper: clamp a float to [0.0, 1.0]
    fn clampf(v: f64) -> f64 {
        if v < 0.0 {
            0.0
        } else if v > 1.0 {
            1.0
        } else {
            v
        }
    }

    // one third = 1.0 / 3.0
    const ONE_THIRD: f64 = 1.0 / 3.0;
    const TWO_THIRD: f64 = 2.0 / 3.0;

    fn hue_to_rgb(m1: f64, m2: f64, mut h: f64) -> f64 {
        if h < 0.0 {
            h += 1.0;
        }
        if h > 1.0 {
            h -= 1.0;
        }
        if h * 6.0 < 1.0 {
            return m1 + (m2 - m1) * h * 6.0;
        }
        if h * 2.0 < 1.0 {
            return m2;
        }
        if h * 3.0 < 2.0 {
            return m1 + (m2 - m1) * (TWO_THIRD - h) * 6.0;
        }
        m1
    }

    cs_func!("rgb_to_hsv", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "rgb_to_hsv() requires 3 arguments (r, g, b)",
            ));
        }
        let r = args[0]
            .as_f64()
            .ok_or_else(|| PyError::type_error("r must be a number"))?;
        let g = args[1]
            .as_f64()
            .ok_or_else(|| PyError::type_error("g must be a number"))?;
        let b = args[2]
            .as_f64()
            .ok_or_else(|| PyError::type_error("b must be a number"))?;

        let maxc = r.max(g).max(b);
        let minc = r.min(g).min(b);
        let v = maxc;
        if minc == maxc {
            return Ok(py_tuple(vec![py_float(0.0), py_float(0.0), py_float(v)]));
        }
        let s = (maxc - minc) / maxc;
        let rc = (maxc - r) / (maxc - minc);
        let gc = (maxc - g) / (maxc - minc);
        let bc = (maxc - b) / (maxc - minc);
        let h = if r == maxc {
            bc - gc
        } else if g == maxc {
            2.0 + rc - bc
        } else {
            4.0 + gc - rc
        };
        let h = (h / 6.0) % 1.0;
        let h = if h < 0.0 { h + 1.0 } else { h };
        Ok(py_tuple(vec![py_float(h), py_float(s), py_float(v)]))
    });

    cs_func!("hsv_to_rgb", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "hsv_to_rgb() requires 3 arguments (h, s, v)",
            ));
        }
        let h = args[0]
            .as_f64()
            .ok_or_else(|| PyError::type_error("h must be a number"))?;
        let s = args[1]
            .as_f64()
            .ok_or_else(|| PyError::type_error("s must be a number"))?;
        let v = args[2]
            .as_f64()
            .ok_or_else(|| PyError::type_error("v must be a number"))?;

        if s == 0.0 {
            let gray = clampf(v);
            return Ok(py_tuple(vec![
                py_float(gray),
                py_float(gray),
                py_float(gray),
            ]));
        }

        let h = (h % 1.0 + 1.0) % 1.0;
        let hi = (h * 6.0).floor() as i32;
        let f = h * 6.0 - hi as f64;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));

        let (r, g, b) = match hi % 6 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        Ok(py_tuple(vec![
            py_float(clampf(r)),
            py_float(clampf(g)),
            py_float(clampf(b)),
        ]))
    });

    // `colorsys.rgb_to_yiq`/`yiq_to_rgb` — were missing entirely
    // (`AttributeError`), breaking `test_colorsys.py`. Formulas copied
    // directly from real CPython's own `Lib/colorsys.py`.
    cs_func!("rgb_to_yiq", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "rgb_to_yiq() requires 3 arguments (r, g, b)",
            ));
        }
        let r = args[0]
            .as_f64()
            .ok_or_else(|| PyError::type_error("r must be a number"))?;
        let g = args[1]
            .as_f64()
            .ok_or_else(|| PyError::type_error("g must be a number"))?;
        let b = args[2]
            .as_f64()
            .ok_or_else(|| PyError::type_error("b must be a number"))?;
        let y = 0.30 * r + 0.59 * g + 0.11 * b;
        let i = 0.74 * (r - y) - 0.27 * (b - y);
        let q = 0.48 * (r - y) + 0.41 * (b - y);
        Ok(py_tuple(vec![py_float(y), py_float(i), py_float(q)]))
    });

    cs_func!("yiq_to_rgb", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "yiq_to_rgb() requires 3 arguments (y, i, q)",
            ));
        }
        let y = args[0]
            .as_f64()
            .ok_or_else(|| PyError::type_error("y must be a number"))?;
        let i = args[1]
            .as_f64()
            .ok_or_else(|| PyError::type_error("i must be a number"))?;
        let q = args[2]
            .as_f64()
            .ok_or_else(|| PyError::type_error("q must be a number"))?;
        let r = y + 0.9468822170900693 * i + 0.6235565819861433 * q;
        let g = y - 0.27478764629897834 * i - 0.6356910791873801 * q;
        let b = y - 1.1085450346420322 * i + 1.7090069284064666 * q;
        Ok(py_tuple(vec![
            py_float(clampf(r)),
            py_float(clampf(g)),
            py_float(clampf(b)),
        ]))
    });

    cs_func!("rgb_to_hls", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "rgb_to_hls() requires 3 arguments (r, g, b)",
            ));
        }
        let r = args[0]
            .as_f64()
            .ok_or_else(|| PyError::type_error("r must be a number"))?;
        let g = args[1]
            .as_f64()
            .ok_or_else(|| PyError::type_error("g must be a number"))?;
        let b = args[2]
            .as_f64()
            .ok_or_else(|| PyError::type_error("b must be a number"))?;

        let maxc = r.max(g).max(b);
        let minc = r.min(g).min(b);
        let l = (minc + maxc) / 2.0;
        if minc == maxc {
            return Ok(py_tuple(vec![py_float(0.0), py_float(l), py_float(0.0)]));
        }
        let s = if l <= 0.5 {
            (maxc - minc) / (maxc + minc)
        } else {
            (maxc - minc) / (2.0 - maxc - minc)
        };
        let rc = (maxc - r) / (maxc - minc);
        let gc = (maxc - g) / (maxc - minc);
        let bc = (maxc - b) / (maxc - minc);
        let h = if r == maxc {
            bc - gc
        } else if g == maxc {
            2.0 + rc - bc
        } else {
            4.0 + gc - rc
        };
        let h = (h / 6.0) % 1.0;
        let h = if h < 0.0 { h + 1.0 } else { h };
        Ok(py_tuple(vec![py_float(h), py_float(l), py_float(s)]))
    });

    cs_func!("hls_to_rgb", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "hls_to_rgb() requires 3 arguments (h, l, s)",
            ));
        }
        let h = args[0]
            .as_f64()
            .ok_or_else(|| PyError::type_error("h must be a number"))?;
        let l = args[1]
            .as_f64()
            .ok_or_else(|| PyError::type_error("l must be a number"))?;
        let s = args[2]
            .as_f64()
            .ok_or_else(|| PyError::type_error("s must be a number"))?;

        if s == 0.0 {
            return Ok(py_tuple(vec![py_float(l), py_float(l), py_float(l)]));
        }
        let m2 = if l <= 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let m1 = 2.0 * l - m2;
        let r = hue_to_rgb(m1, m2, h + ONE_THIRD);
        let g = hue_to_rgb(m1, m2, h);
        let b = hue_to_rgb(m1, m2, h - ONE_THIRD);
        Ok(py_tuple(vec![
            py_float(clampf(r)),
            py_float(clampf(g)),
            py_float(clampf(b)),
        ]))
    });

    d
}
