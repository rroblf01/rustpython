#!/usr/bin/env python3
"""
Second pass: Fix remaining issues after the initial refactor.

1. Make helper functions pub in object.rs so modules can use them
2. Add use crate::modules::*; to object.rs (for create_builtins usage inside object.rs)
3. Add missing std imports to each module file
"""

import os
import re

PROJECT = "/opt/data/proyectos/rustpython"
OBJECT_RS = os.path.join(PROJECT, "src", "object.rs")
MODULES_DIR = os.path.join(PROJECT, "src", "modules")

def read_file(path):
    with open(path, 'r') as f:
        return f.read()

def write_file(path, content):
    with open(path, 'w') as f:
        f.write(content)

# Helper functions (non-pub) in object.rs used by module files
# These need to be made pub
HELPER_FUNCTIONS = [
    "io_stringio_read",
    "io_stringio_readline", 
    "io_stringio_write",
    "io_stringio_seek",
    "io_stringio_tell",
    "io_stringio_getvalue",
    "io_bytesio_read",
    "io_bytesio_readline",
    "io_bytesio_write",
    "io_bytesio_seek",
    "io_bytesio_tell",
    "io_bytesio_getvalue",
    "logging_debug",
    "logging_info",
    "logging_warning",
    "logging_error",
    "zipfile_constructor",
    "shelf_open",
    "gzip_crc32",
    "deepcopy_one",
    "ENUM_AUTO_COUNTER",
    "PATH_TYPE",
    "LOG_LEVEL",
    "mime_guess_type",
    "mime_guess_extension",
    "mime_add_type",
    "fast_random_f64",
    "fast_random_u64",
    "json_encode_full",
    "json_decode",
]

def make_helpers_pub():
    """Make helper functions pub in object.rs."""
    text = read_file(OBJECT_RS)
    lines = text.split('\n')
    modified = False
    
    for name in HELPER_FUNCTIONS:
        for i, line in enumerate(lines):
            # Match: fn <name>(  (without pub)
            pattern = r'^fn ' + re.escape(name) + r'\('
            if re.match(pattern, line):
                lines[i] = 'pub ' + line
                print(f"Made pub: {name} at line {i+1}")
                modified = True
                break
            # Match: fn <name><  (generic)
            pattern2 = r'^fn ' + re.escape(name) + r'<'
            if re.match(pattern2, line):
                lines[i] = 'pub ' + line
                print(f"Made pub (generic): {name} at line {i+1}")
                modified = True
                break
    
    if modified:
        write_file(OBJECT_RS, '\n'.join(lines))
        print("Updated object.rs with pub helpers")
    else:
        print("No helpers needed to be made pub")

def add_modules_import_to_object():
    """Add use crate::modules::*; to object.rs imports."""
    text = read_file(OBJECT_RS)
    lines = text.split('\n')
    
    # Check if already present
    if any("use crate::modules::*;" in line for line in lines):
        print("object.rs already has modules import")
        return
    
    # Add after the existing imports
    for i, line in enumerate(lines):
        if "use crate::bytecode" in line and "needs_arg" in line:
            lines.insert(i + 1, "use crate::modules::*;")
            write_file(OBJECT_RS, '\n'.join(lines))
            print(f"Added use crate::modules::*; to object.rs at line {i+2}")
            return
    
    print("Could not find import line in object.rs")

# Additional imports needed by each module file
MODULE_IMPORTS = {
    "core": [
        "use std::rc::Rc;",
        "use std::cell::RefCell;",
        "use num_traits::ToPrimitive;",
    ],
    "crypto": [
        "use std::rc::Rc;",
        "use std::cell::RefCell;",
    ],
    "text": [
        "use std::rc::Rc;",
        "use std::cell::RefCell;",
        "use num_traits::ToPrimitive;",
    ],
    "data": [
        "use std::rc::Rc;",
        "use std::cell::RefCell;",
        "use num_traits::ToPrimitive;",
        "use num_bigint::BigInt;",
        "use std::sync::atomic::{AtomicI64, Ordering};",
    ],
    "net": [
        "use std::rc::Rc;",
        "use std::cell::RefCell;",
        "use std::process::Command;",
    ],
    "dev": [
        "use std::rc::Rc;",
        "use std::cell::RefCell;",
    ],
    "files": [
        "use std::rc::Rc;",
        "use std::cell::RefCell;",
        "use std::sync::atomic::{AtomicI64, Ordering};",
        "use num_traits::ToPrimitive;",
        "use crate::bytecode::{needs_arg, CodeObject};",
    ],
    "misc": [
        "use std::rc::Rc;",
        "use std::cell::RefCell;",
        "use std::sync::{Arc, Mutex};",
        "use num_traits::ToPrimitive;",
        "use num_bigint::BigInt;",
        "use std::sync::atomic::{AtomicI64, Ordering};",
        "use crate::bytecode::{needs_arg, CodeObject};",
    ],
}

def add_module_imports():
    """Add missing imports to each module file."""
    for cat, imports in MODULE_IMPORTS.items():
        filepath = os.path.join(MODULES_DIR, f"{cat}.rs")
        if not os.path.exists(filepath):
            print(f"Module file not found: {filepath}")
            continue
        
        text = read_file(filepath)
        lines = text.split('\n')
        
        # Find where to insert imports (after the existing use lines)
        insert_at = 0
        for i, line in enumerate(lines):
            if line.startswith("use ") and not line.startswith("use crate::object"):
                # Skip already present imports
                continue
            if line.strip() == "":
                insert_at = i + 1
            elif insert_at == 0:
                insert_at = i + 1
        
        if insert_at == 0:
            insert_at = 2  # After header comment
        
        # Filter out imports already present
        new_imports = []
        for imp in imports:
            if not any(imp in line for line in lines):
                new_imports.append(imp)
        
        if new_imports:
            # Insert after the existing use lines
            text_lines = lines[:insert_at] + new_imports + lines[insert_at:]
            write_file(filepath, '\n'.join(text_lines))
            print(f"Added {len(new_imports)} imports to {cat}.rs")

def main():
    make_helpers_pub()
    add_modules_import_to_object()
    add_module_imports()
    print("\nDone fixing imports and visibility!")

if __name__ == "__main__":
    main()
