use crate::object::*;
use std::collections::HashMap;

pub fn create_html_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! html_func {
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

    html_func!("escape", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("escape() missing required argument"));
        }
        let s = args[0].str();
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '&' => result.push_str("&amp;"),
                '<' => result.push_str("&lt;"),
                '>' => result.push_str("&gt;"),
                '"' => result.push_str("&quot;"),
                '\'' => result.push_str("&#x27;"),
                _ => result.push(c),
            }
        }
        Ok(py_str(&result))
    });

    html_func!("unescape", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("unescape() missing required argument"));
        }
        let s = args[0].str();
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len();
        let mut result = String::with_capacity(s.len());
        let mut i = 0;
        while i < len {
            if chars[i] == '&' {
                // Find the closing semicolon
                if let Some(end) = chars[i..].iter().position(|&c| c == ';') {
                    let entity: String = chars[i + 1..i + end].iter().collect();
                    let decoded: Option<String> = match entity.as_str() {
                        "amp" => Some("&".to_string()),
                        "lt" => Some("<".to_string()),
                        "gt" => Some(">".to_string()),
                        "quot" => Some("\"".to_string()),
                        "#x27" | "#39" => Some("'".to_string()),
                        "nbsp" => Some("\u{00A0}".to_string()),
                        _ => {
                            // Try numeric character reference
                            if entity.starts_with('#') {
                                let codepoint: Option<u32> =
                                    if entity.starts_with("#x") || entity.starts_with("#X") {
                                        u32::from_str_radix(&entity[2..], 16).ok()
                                    } else {
                                        entity[1..].parse().ok()
                                    };
                                codepoint
                                    .and_then(|cp| char::from_u32(cp))
                                    .map(|c| c.to_string())
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(replacement) = decoded {
                        result.push_str(&replacement);
                        i += end + 1;
                        continue;
                    }
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        Ok(py_str(&result))
    });

    d
}

pub fn create_html_entities_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Build the html5 dict of entity name -> character
    let pairs: &[(&str, &str)] = &[
        ("amp", "&"),
        ("lt", "<"),
        ("gt", ">"),
        ("quot", "\""),
        ("apos", "'"),
        ("nbsp", "\u{00A0}"),
        ("iexcl", "\u{00A1}"),
        ("cent", "\u{00A2}"),
        ("pound", "\u{00A3}"),
        ("curren", "\u{00A4}"),
        ("yen", "\u{00A5}"),
        ("brvbar", "\u{00A6}"),
        ("sect", "\u{00A7}"),
        ("uml", "\u{00A8}"),
        ("copy", "\u{00A9}"),
        ("ordf", "\u{00AA}"),
        ("laquo", "\u{00AB}"),
        ("not", "\u{00AC}"),
        ("shy", "\u{00AD}"),
        ("reg", "\u{00AE}"),
        ("macr", "\u{00AF}"),
        ("deg", "\u{00B0}"),
        ("plusmn", "\u{00B1}"),
        ("sup2", "\u{00B2}"),
        ("sup3", "\u{00B3}"),
        ("acute", "\u{00B4}"),
        ("micro", "\u{00B5}"),
        ("para", "\u{00B6}"),
        ("middot", "\u{00B7}"),
        ("cedil", "\u{00B8}"),
        ("sup1", "\u{00B9}"),
        ("ordm", "\u{00BA}"),
        ("raquo", "\u{00BB}"),
        ("frac14", "\u{00BC}"),
        ("frac12", "\u{00BD}"),
        ("frac34", "\u{00BE}"),
        ("iquest", "\u{00BF}"),
        ("times", "\u{00D7}"),
        ("divide", "\u{00F7}"),
        ("OElig", "\u{0152}"),
        ("oelig", "\u{0153}"),
        ("Scaron", "\u{0160}"),
        ("scaron", "\u{0161}"),
        ("Yuml", "\u{0178}"),
        ("fnof", "\u{0192}"),
        ("circ", "\u{02C6}"),
        ("tilde", "\u{02DC}"),
        ("Alpha", "\u{0391}"),
        ("Beta", "\u{0392}"),
        ("Gamma", "\u{0393}"),
        ("Delta", "\u{0394}"),
        ("Epsilon", "\u{0395}"),
        ("Zeta", "\u{0396}"),
        ("Eta", "\u{0397}"),
        ("Theta", "\u{0398}"),
        ("Iota", "\u{0399}"),
        ("Kappa", "\u{039A}"),
        ("Lambda", "\u{039B}"),
        ("Mu", "\u{039C}"),
        ("Nu", "\u{039D}"),
        ("Xi", "\u{039E}"),
        ("Omicron", "\u{039F}"),
        ("Pi", "\u{03A0}"),
        ("Rho", "\u{03A1}"),
        ("Sigma", "\u{03A3}"),
        ("Tau", "\u{03A4}"),
        ("Upsilon", "\u{03A5}"),
        ("Phi", "\u{03A6}"),
        ("Chi", "\u{03A7}"),
        ("Psi", "\u{03A8}"),
        ("Omega", "\u{03A9}"),
        ("alpha", "\u{03B1}"),
        ("beta", "\u{03B2}"),
        ("gamma", "\u{03B3}"),
        ("delta", "\u{03B4}"),
        ("epsilon", "\u{03B5}"),
        ("zeta", "\u{03B6}"),
        ("eta", "\u{03B7}"),
        ("theta", "\u{03B8}"),
        ("iota", "\u{03B9}"),
        ("kappa", "\u{03BA}"),
        ("lambda", "\u{03BB}"),
        ("mu", "\u{03BC}"),
        ("nu", "\u{03BD}"),
        ("xi", "\u{03BE}"),
        ("omicron", "\u{03BF}"),
        ("pi", "\u{03C0}"),
        ("rho", "\u{03C1}"),
        ("sigmaf", "\u{03C2}"),
        ("sigma", "\u{03C3}"),
        ("tau", "\u{03C4}"),
        ("upsilon", "\u{03C5}"),
        ("phi", "\u{03C6}"),
        ("chi", "\u{03C7}"),
        ("psi", "\u{03C8}"),
        ("omega", "\u{03C9}"),
        ("thetasym", "\u{03D1}"),
        ("upsih", "\u{03D2}"),
        ("piv", "\u{03D6}"),
        ("ensp", "\u{2002}"),
        ("emsp", "\u{2003}"),
        ("thinsp", "\u{2009}"),
        ("zwnj", "\u{200C}"),
        ("zwj", "\u{200D}"),
        ("lrm", "\u{200E}"),
        ("rlm", "\u{200F}"),
        ("ndash", "\u{2013}"),
        ("mdash", "\u{2014}"),
        ("lsquo", "\u{2018}"),
        ("rsquo", "\u{2019}"),
        ("sbquo", "\u{201A}"),
        ("ldquo", "\u{201C}"),
        ("rdquo", "\u{201D}"),
        ("bdquo", "\u{201E}"),
        ("dagger", "\u{2020}"),
        ("Dagger", "\u{2021}"),
        ("bull", "\u{2022}"),
        ("hellip", "\u{2026}"),
        ("permil", "\u{2030}"),
        ("prime", "\u{2032}"),
        ("Prime", "\u{2033}"),
        ("lsaquo", "\u{2039}"),
        ("rsaquo", "\u{203A}"),
        ("oline", "\u{203E}"),
        ("frasl", "\u{2044}"),
        ("euro", "\u{20AC}"),
        ("image", "\u{2111}"),
        ("weierp", "\u{2118}"),
        ("real", "\u{211C}"),
        ("trade", "\u{2122}"),
        ("alefsym", "\u{2135}"),
        ("larr", "\u{2190}"),
        ("uarr", "\u{2191}"),
        ("rarr", "\u{2192}"),
        ("darr", "\u{2193}"),
        ("harr", "\u{2194}"),
        ("crarr", "\u{21B5}"),
        ("lArr", "\u{21D0}"),
        ("uArr", "\u{21D1}"),
        ("rArr", "\u{21D2}"),
        ("dArr", "\u{21D3}"),
        ("hArr", "\u{21D4}"),
        ("forall", "\u{2200}"),
        ("part", "\u{2202}"),
        ("exist", "\u{2203}"),
        ("empty", "\u{2205}"),
        ("nabla", "\u{2207}"),
        ("isin", "\u{2208}"),
        ("notin", "\u{2209}"),
        ("ni", "\u{220B}"),
        ("prod", "\u{220F}"),
        ("sum", "\u{2211}"),
        ("minus", "\u{2212}"),
        ("lowast", "\u{2217}"),
        ("radic", "\u{221A}"),
        ("prop", "\u{221D}"),
        ("infin", "\u{221E}"),
        ("ang", "\u{2220}"),
        ("and", "\u{2227}"),
        ("or", "\u{2228}"),
        ("cap", "\u{2229}"),
        ("cup", "\u{222A}"),
        ("int", "\u{222B}"),
        ("there4", "\u{2234}"),
        ("sim", "\u{223C}"),
        ("cong", "\u{2245}"),
        ("asymp", "\u{2248}"),
        ("ne", "\u{2260}"),
        ("equiv", "\u{2261}"),
        ("le", "\u{2264}"),
        ("ge", "\u{2265}"),
        ("sub", "\u{2282}"),
        ("sup", "\u{2283}"),
        ("nsub", "\u{2284}"),
        ("sube", "\u{2286}"),
        ("supe", "\u{2287}"),
        ("oplus", "\u{2295}"),
        ("otimes", "\u{2297}"),
        ("perp", "\u{22A5}"),
        ("sdot", "\u{22C5}"),
        ("lceil", "\u{2308}"),
        ("rceil", "\u{2309}"),
        ("lfloor", "\u{230A}"),
        ("rfloor", "\u{230B}"),
        ("lang", "\u{2329}"),
        ("rang", "\u{232A}"),
        ("loz", "\u{25CA}"),
        ("spades", "\u{2660}"),
        ("clubs", "\u{2663}"),
        ("hearts", "\u{2665}"),
        ("diams", "\u{2666}"),
    ];

    let py_dict_obj = py_dict();
    if let PyObject::Dict(ref mut pd) = &mut *py_dict_obj.borrow_mut() {
        for (name, ch) in pairs {
            pd.set(py_str(name), py_str(ch)).ok();
        }
    }

    d.insert_str("html5", py_dict_obj);
    d
}
