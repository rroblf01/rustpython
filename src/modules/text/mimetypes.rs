use crate::object::*;
use once_cell::sync::Lazy;
use std::collections::HashMap;

// Moved here from object.rs (was under a "=== MIMETYPES MODULE ===" banner
// in the monolithic object.rs — see the file-splitting refactor's memory
// entry for context).
// Static MIME type database: extension -> (type, encoding)
static KNOWN_TYPES: Lazy<HashMap<String, (String, String)>> = Lazy::new(|| {
    HashMap::from([
        (
            ".html".to_string(),
            ("text/html".to_string(), "".to_string()),
        ),
        (
            ".htm".to_string(),
            ("text/html".to_string(), "".to_string()),
        ),
        (".css".to_string(), ("text/css".to_string(), "".to_string())),
        (
            ".js".to_string(),
            ("application/javascript".to_string(), "".to_string()),
        ),
        (
            ".json".to_string(),
            ("application/json".to_string(), "".to_string()),
        ),
        (
            ".xml".to_string(),
            ("application/xml".to_string(), "".to_string()),
        ),
        (
            ".txt".to_string(),
            ("text/plain".to_string(), "".to_string()),
        ),
        (".csv".to_string(), ("text/csv".to_string(), "".to_string())),
        (
            ".md".to_string(),
            ("text/markdown".to_string(), "".to_string()),
        ),
        (
            ".py".to_string(),
            ("text/x-python".to_string(), "".to_string()),
        ),
        (
            ".png".to_string(),
            ("image/png".to_string(), "".to_string()),
        ),
        (
            ".jpg".to_string(),
            ("image/jpeg".to_string(), "".to_string()),
        ),
        (
            ".jpeg".to_string(),
            ("image/jpeg".to_string(), "".to_string()),
        ),
        (
            ".gif".to_string(),
            ("image/gif".to_string(), "".to_string()),
        ),
        (
            ".bmp".to_string(),
            ("image/bmp".to_string(), "".to_string()),
        ),
        (
            ".ico".to_string(),
            ("image/x-icon".to_string(), "".to_string()),
        ),
        (
            ".svg".to_string(),
            ("image/svg+xml".to_string(), "".to_string()),
        ),
        (
            ".webp".to_string(),
            ("image/webp".to_string(), "".to_string()),
        ),
        (
            ".mp3".to_string(),
            ("audio/mpeg".to_string(), "".to_string()),
        ),
        (
            ".wav".to_string(),
            ("audio/wav".to_string(), "".to_string()),
        ),
        (
            ".ogg".to_string(),
            ("audio/ogg".to_string(), "".to_string()),
        ),
        (
            ".mp4".to_string(),
            ("video/mp4".to_string(), "".to_string()),
        ),
        (
            ".webm".to_string(),
            ("video/webm".to_string(), "".to_string()),
        ),
        (
            ".avi".to_string(),
            ("video/x-msvideo".to_string(), "".to_string()),
        ),
        (
            ".mov".to_string(),
            ("video/quicktime".to_string(), "".to_string()),
        ),
        (
            ".pdf".to_string(),
            ("application/pdf".to_string(), "".to_string()),
        ),
        (
            ".zip".to_string(),
            ("application/zip".to_string(), "".to_string()),
        ),
        (
            ".gz".to_string(),
            ("application/gzip".to_string(), "".to_string()),
        ),
        (
            ".tar".to_string(),
            ("application/x-tar".to_string(), "".to_string()),
        ),
        (
            ".rar".to_string(),
            ("application/vnd.rar".to_string(), "".to_string()),
        ),
        (
            ".7z".to_string(),
            ("application/x-7z-compressed".to_string(), "".to_string()),
        ),
        (
            ".exe".to_string(),
            ("application/x-msdownload".to_string(), "".to_string()),
        ),
        (
            ".bin".to_string(),
            ("application/octet-stream".to_string(), "".to_string()),
        ),
        (
            ".wasm".to_string(),
            ("application/wasm".to_string(), "".to_string()),
        ),
        (
            ".woff".to_string(),
            ("font/woff".to_string(), "".to_string()),
        ),
        (
            ".woff2".to_string(),
            ("font/woff2".to_string(), "".to_string()),
        ),
        (".ttf".to_string(), ("font/ttf".to_string(), "".to_string())),
        (".otf".to_string(), ("font/otf".to_string(), "".to_string())),
        (
            ".yaml".to_string(),
            ("text/yaml".to_string(), "".to_string()),
        ),
        (
            ".yml".to_string(),
            ("text/yaml".to_string(), "".to_string()),
        ),
        (
            ".toml".to_string(),
            ("application/toml".to_string(), "".to_string()),
        ),
        (
            ".doc".to_string(),
            ("application/msword".to_string(), "".to_string()),
        ),
        (
            ".docx".to_string(),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    .to_string(),
                "".to_string(),
            ),
        ),
        (
            ".xls".to_string(),
            ("application/vnd.ms-excel".to_string(), "".to_string()),
        ),
        (
            ".xlsx".to_string(),
            (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
                "".to_string(),
            ),
        ),
        (
            ".ppt".to_string(),
            ("application/vnd.ms-powerpoint".to_string(), "".to_string()),
        ),
        (
            ".pptx".to_string(),
            (
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                    .to_string(),
                "".to_string(),
            ),
        ),
        (
            ".rtf".to_string(),
            ("application/rtf".to_string(), "".to_string()),
        ),
    ])
});

// Static reverse mapping: type -> extension
static KNOWN_EXTS: Lazy<HashMap<String, String>> = Lazy::new(|| {
    HashMap::from([
        ("text/html".to_string(), ".html".to_string()),
        ("text/css".to_string(), ".css".to_string()),
        ("application/javascript".to_string(), ".js".to_string()),
        ("application/json".to_string(), ".json".to_string()),
        ("application/xml".to_string(), ".xml".to_string()),
        ("text/plain".to_string(), ".txt".to_string()),
        ("text/csv".to_string(), ".csv".to_string()),
        ("text/markdown".to_string(), ".md".to_string()),
        ("text/x-python".to_string(), ".py".to_string()),
        ("image/png".to_string(), ".png".to_string()),
        ("image/jpeg".to_string(), ".jpg".to_string()),
        ("image/gif".to_string(), ".gif".to_string()),
        ("image/bmp".to_string(), ".bmp".to_string()),
        ("image/x-icon".to_string(), ".ico".to_string()),
        ("image/svg+xml".to_string(), ".svg".to_string()),
        ("image/webp".to_string(), ".webp".to_string()),
        ("audio/mpeg".to_string(), ".mp3".to_string()),
        ("audio/wav".to_string(), ".wav".to_string()),
        ("audio/ogg".to_string(), ".ogg".to_string()),
        ("video/mp4".to_string(), ".mp4".to_string()),
        ("video/webm".to_string(), ".webm".to_string()),
        ("video/x-msvideo".to_string(), ".avi".to_string()),
        ("video/quicktime".to_string(), ".mov".to_string()),
        ("application/pdf".to_string(), ".pdf".to_string()),
        ("application/zip".to_string(), ".zip".to_string()),
        ("application/gzip".to_string(), ".gz".to_string()),
        ("application/x-tar".to_string(), ".tar".to_string()),
        ("application/vnd.rar".to_string(), ".rar".to_string()),
        ("application/x-7z-compressed".to_string(), ".7z".to_string()),
        ("application/x-msdownload".to_string(), ".exe".to_string()),
        ("application/octet-stream".to_string(), ".bin".to_string()),
        ("application/wasm".to_string(), ".wasm".to_string()),
        ("font/woff".to_string(), ".woff".to_string()),
        ("font/woff2".to_string(), ".woff2".to_string()),
        ("font/ttf".to_string(), ".ttf".to_string()),
        ("font/otf".to_string(), ".otf".to_string()),
        ("text/yaml".to_string(), ".yaml".to_string()),
        ("application/toml".to_string(), ".toml".to_string()),
        ("application/msword".to_string(), ".doc".to_string()),
        (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string(),
            ".docx".to_string(),
        ),
        ("application/vnd.ms-excel".to_string(), ".xls".to_string()),
        (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
            ".xlsx".to_string(),
        ),
        (
            "application/vnd.ms-powerpoint".to_string(),
            ".ppt".to_string(),
        ),
        (
            "application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string(),
            ".pptx".to_string(),
        ),
        ("application/rtf".to_string(), ".rtf".to_string()),
    ])
});

pub fn mime_guess_type(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "guess_type() takes at least 1 argument",
        ));
    }
    let url = args[0].str();
    // Strip query string and fragment
    let path = url
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("");
    let ext = {
        let p = path.rfind('.').map(|i| &path[i..]).unwrap_or("");
        p.to_lowercase()
    };
    let (mime_type, encoding) = KNOWN_TYPES
        .get(&ext)
        .cloned()
        .unwrap_or_else(|| ("application/octet-stream".to_string(), "".to_string()));
    let encoding = if encoding.is_empty() {
        py_none()
    } else {
        py_str(&encoding)
    };
    let result = PyObjectRef::new(PyObject::Tuple(vec![py_str(&mime_type), encoding]));
    Ok(result)
}

pub fn mime_guess_extension(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "guess_extension() takes at least 1 argument",
        ));
    }
    let mime_type = args[0].str();
    let ext = KNOWN_EXTS.get(&mime_type);
    match ext {
        Some(e) => Ok(py_str(e)),
        None => Ok(py_none()),
    }
}

pub fn mime_add_type(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "add_type() takes at least 2 arguments (type, ext)",
        ));
    }
    let _ = args;
    Ok(py_none())
}

pub fn create_mimetypes_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "guess_type",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "guess_type".to_string(),
            func: mime_guess_type,
        }),
    );
    d.insert_str(
        "guess_extension",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "guess_extension".to_string(),
            func: mime_guess_extension,
        }),
    );
    d.insert_str(
        "add_type",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "add_type".to_string(),
            func: mime_add_type,
        }),
    );
    // list of known types, init, read_mime_types, etc. can be added as needed
    d.insert_str("known_types", py_dict());
    d.insert_str("knownfiles", py_list(vec![]));
    d.insert_str("inited", py_bool(false));
    d
}
