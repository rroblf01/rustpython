#!/usr/bin/env python3
"""
Refactor src/object.rs: Extract create_*_dict functions into category module files.

Reads src/object.rs, finds all pub fn create_* functions (including create_builtins),
extracts them by brace-matching, categorizes them, writes to src/modules/<category>.rs,
creates src/modules/mod.rs, and removes them from object.rs.
"""

import os
import re

PROJECT = "/opt/data/proyectos/rustpython"
OBJECT_RS = os.path.join(PROJECT, "src", "object.rs")
MODULES_DIR = os.path.join(PROJECT, "src", "modules")
VM_RS = os.path.join(PROJECT, "src", "vm.rs")

# Category mapping: function name -> category file
CATEGORIES = {
    # core
    "create_builtins": "core",
    "create_sys_dict": "core",
    "create_math_dict": "core",
    "create_os_dict": "core",
    "create_operator_dict": "core",
    # crypto
    "create_hashlib_dict": "crypto",
    "create_base64_dict": "crypto",
    "create_secrets_dict": "crypto",
    "create_hmac_dict": "crypto",
    "create_zlib_dict": "crypto",
    # text
    "create_string_dict": "text",
    "create_string_dict_v2": "text",
    "create_textwrap_dict": "text",
    "create_pprint_dict": "text",
    "create_reprlib_dict": "text",
    "create_difflib_dict": "text",
    "create_mimetypes_dict": "text",
    # data
    "create_datetime_dict": "data",
    "create_calendar_dict": "data",
    "create_collections_dict": "data",
    "create_itertools_dict": "data",
    "create_functools_dict": "data",
    "create_random_dict": "data",
    "create_statistics_dict": "data",
    "create_decimal_dict": "data",
    "create_fractions_dict": "data",
    "create_json_dict": "data",
    # net
    "create_socket_dict": "net",
    "create_select_dict": "net",
    "create_http_dict": "net",
    "create_html_dict": "net",
    "create_html_entities_dict": "net",
    "create_subprocess_dict": "net",
    "create_urllib_dict": "net",
    # dev
    "create_typing_dict": "dev",
    "create_dis_dict": "dev",
    "create_unittest_dict": "dev",
    "create_pdb_dict": "dev",
    "create_traceback_dict": "dev",
    "create_warnings_dict": "dev",
    "create_abc_dict": "dev",
    "create_dataclasses_dict": "dev",
    # files
    "create_shutil_dict": "files",
    "create_tempfile_dict": "files",
    "create_glob_dict": "files",
    "create_fnmatch_dict": "files",
    "create_linecache_dict": "files",
    "create_pathlib_dict": "files",
    "create_zipfile_dict": "files",
    "create_gzip_dict": "files",
    "create_tarfile_dict": "files",
    "create_shelve_dict": "files",
    # misc
    "create_threading_dict": "misc",
    "create_thread_module_dict": "misc",
    "create_signal_dict": "misc",
    "create_gc_dict": "misc",
    "create_sysconfig_dict": "misc",
    "create_weakref_dict": "misc",
    "create_weakref_weak_val_dict": "misc",
    "create_weakref_weak_key_dict": "misc",
    "create_weakref_weak_set": "misc",
    "create_collections_abc_dict": "misc",
    "create_copy_dict": "misc",
    "create_types_dict": "misc",
    "create_struct_dict": "misc",
    "create_bisect_dict": "misc",
    "create_heapq_dict": "misc",
    "create_enum_dict": "misc",
    "create_contextlib_dict": "misc",
    "create_timeit_dict": "misc",
    "create_pickle_dict": "misc",
    "create_logging_dict": "misc",
    "create_csv_dict": "misc",
    "create_io_dict": "misc",
    "create_getopt_dict": "misc",
    "create_getpass_dict": "misc",
    "create_platform_dict": "misc",
    "create_locale_dict": "misc",
    "create_cmath_dict": "misc",
    "create_hashlib_extra_dict": "misc",
    "create_json_tool_dict": "misc",
    "create_graphlib_dict": "misc",
    "create_array_dict": "misc",
    "create_colorsys_dict": "misc",
    "create_wave_dict": "misc",
    "create_queue_dict": "misc",
    "create_uuid_dict": "misc",
    "create_re_dict": "misc",
}


def read_file(path):
    with open(path, 'r') as f:
        return f.read()


def write_file(path, content):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w') as f:
        f.write(content)


def find_function_ranges(text):
    """
    Find all pub fn create_* functions and their line ranges.
    Uses brace matching to find the end of each function.
    Returns list of (name, start_line, end_line) (0-indexed lines).
    """
    lines = text.split('\n')
    funcs = []

    # First pass: find all function start lines
    func_starts = []  # list of (func_name, line_idx)
    for i, line in enumerate(lines):
        m = re.match(r'^pub fn (create_\w+)\(', line)
        if m:
            name = m.group(1)
            func_starts.append((name, i))

    # For each function, find its end by brace matching
    for func_name, start_line in func_starts:
        # Find opening brace (may be on same line or next few lines)
        brace_index = None
        for j in range(start_line, min(start_line + 5, len(lines))):
            if '{' in lines[j]:
                brace_index = j
                break
        
        if brace_index is None:
            print(f"WARNING: Could not find opening brace for {func_name} at line {start_line}")
            continue

        # Find matching closing brace
        depth = 0
        started = False
        end_line = start_line
        for j in range(start_line, len(lines)):
            # Count braces in this line
            opens = lines[j].count('{')
            closes = lines[j].count('}')
            if not started:
                if opens > 0:
                    started = True
                    depth += opens - closes
                    if depth <= 0:
                        end_line = j
                        break
            else:
                depth += opens - closes
                if depth <= 0:
                    end_line = j
                    break
        
        if not started:
            print(f"WARNING: Could not find brace for {func_name}")
            continue

        funcs.append((func_name, start_line, end_line))

    return funcs


def categorize(funcs):
    """Categorize functions by name using CATEGORIES dict."""
    categorized = {}
    for cat in ["core", "crypto", "text", "data", "net", "dev", "files", "misc"]:
        categorized[cat] = []

    for func_name, start, end in funcs:
        cat = CATEGORIES.get(func_name)
        if cat:
            categorized[cat].append((func_name, start, end))
        else:
            print(f"WARNING: No category for {func_name}")

    return categorized


def build_mod_rs(categorized):
    """Build mod.rs content."""
    lines = []
    for cat in ["core", "crypto", "text", "data", "net", "dev", "files", "misc"]:
        func_list = categorized.get(cat, [])
        if func_list:
            lines.append(f"mod {cat};")
            lines.append(f"pub use {cat}::*;")
    return '\n'.join(lines) + '\n'


def build_category_file(func_texts):
    """Build a category file with imports and functions."""
    header = """use crate::object::*;
use std::collections::HashMap;

"""
    body = '\n\n'.join(func_texts)
    return header + body + '\n'


def main():
    text = read_file(OBJECT_RS)
    lines = text.split('\n')

    funcs = find_function_ranges(text)
    print(f"Found {len(funcs)} functions")

    for func_name, start, end in funcs:
        print(f"  {func_name}: lines {start}-{end}")

    categorized = categorize(funcs)

    # Build grouped function texts and write category files
    for cat in ["core", "crypto", "text", "data", "net", "dev", "files", "misc"]:
        func_list = categorized.get(cat, [])
        if not func_list:
            continue
        
        # Sort by start line
        func_list.sort(key=lambda x: x[1])
        
        texts = []
        for func_name, start, end in func_list:
            text_block = '\n'.join(lines[start:end+1])
            texts.append(text_block)
        
        content = build_category_file(texts)
        filepath = os.path.join(MODULES_DIR, f"{cat}.rs")
        write_file(filepath, content)
        print(f"Wrote {filepath} ({len(texts)} functions)")

    # Write mod.rs
    mod_rs = build_mod_rs(categorized)
    write_file(os.path.join(MODULES_DIR, "mod.rs"), mod_rs)
    print(f"Wrote {os.path.join(MODULES_DIR, 'mod.rs')}")

    # Remove functions from object.rs
    # Build list of line ranges to remove (with leading comment blocks)
    ranges_to_remove = []
    for func_name, start, end in funcs:
        cat = CATEGORIES.get(func_name)
        if not cat:
            continue
        # Include comment block and blank lines immediately before the function
        adj_start = start
        while adj_start > 0:
            prev_line = lines[adj_start - 1].strip()
            if prev_line.startswith('//') or prev_line == '':
                adj_start -= 1
            else:
                break
        # Also include any comment-only lines and blank lines between sections
        while adj_start > 0:
            prev_line = lines[adj_start - 1].strip()
            if prev_line.startswith('//') or prev_line == '':
                adj_start -= 1
            else:
                break
        ranges_to_remove.append((adj_start, end))

    # Merge overlapping ranges
    ranges_to_remove.sort(key=lambda x: x[0])
    merged_ranges = []
    for r in ranges_to_remove:
        if merged_ranges and r[0] <= merged_ranges[-1][1] + 1:
            merged_ranges[-1] = (merged_ranges[-1][0], max(merged_ranges[-1][1], r[1]))
        else:
            merged_ranges.append(r)

    # Remove from bottom up to preserve line numbers
    merged_ranges.sort(key=lambda x: x[0], reverse=True)
    
    new_lines = list(lines)
    for start, end in merged_ranges:
        # Keep end inclusive
        new_lines = new_lines[:start] + new_lines[end+1:]

    new_text = '\n'.join(new_lines)
    write_file(OBJECT_RS, new_text)
    
    removed_count = sum(1 for _, _, _ in funcs if CATEGORIES.get(funcs[0][0]))
    print(f"Removed {removed_count} functions from object.rs ({len(lines)} -> {len(new_lines)} lines)")

    # Update vm.rs imports
    vm_text = read_file(VM_RS)
    # Replace `use crate::object::*;` usage with both imports
    # vm.rs uses create_module (stays in object) AND create_*_dict (now in modules)
    old_import = "use crate::object::*;\nuse crate::jit::JitCompiler;"
    new_import = "use crate::modules::*;\nuse crate::object::*;\nuse crate::jit::JitCompiler;"
    if old_import in vm_text:
        vm_text = vm_text.replace(old_import, new_import)
        write_file(VM_RS, vm_text)
        print("Updated vm.rs imports (added use crate::modules::*)")
    else:
        # Try the existing line
        if "use crate::object::*;" in vm_text and "use crate::jit::JitCompiler;" in vm_text:
            # The import exists but maybe on different lines
            vm_lines = vm_text.split('\n')
            for i, line in enumerate(vm_lines):
                if line.strip() == "use crate::object::*;":
                    vm_lines.insert(i, "use crate::modules::*;")
                    break
            vm_text = '\n'.join(vm_lines)
            write_file(VM_RS, vm_text)
            print("Updated vm.rs imports (inserted use crate::modules::*)")

    print("\nDone! Now run 'cargo check' to verify.")


if __name__ == "__main__":
    main()
