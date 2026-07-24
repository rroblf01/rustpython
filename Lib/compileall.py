"""Byte-compile Python libraries."""

import os
import py_compile


def compile_dir(dir, maxlevels=10, ddir=None, force=False, rx=None, quiet=0,
                legacy=False, optimize=-1, workers=1, invalidation_mode=None,
                stripdir=None, prependdir=None, limit_sl_dest=None,
                hardlink_dupes=False):
    """Byte-compile all modules in the given directory tree."""
    files = []
    for root, dirs, filenames in os.walk(dir):
        for fn in filenames:
            if fn.endswith('.py'):
                files.append(os.path.join(root, fn))
    success = True
    for file in files:
        try:
            py_compile.compile(file, ddir=ddir)
        except py_compile.PyCompileError:
            success = False
    return success


def compile_path(skip_curdir=True, maxlevels=0, force=False, quiet=0,
                 legacy=False, optimize=-1, invalidation_mode=None):
    """Byte-compile all modules on sys.path."""
    import sys
    success = True
    for dir in sys.path:
        if (not dir or dir == os.curdir) and skip_curdir:
            continue
        success &= compile_dir(dir, maxlevels)
    return success


def compile_file(fullname, ddir=None, force=False, rx=None, quiet=0,
                 legacy=False, optimize=-1, invalidation_mode=None):
    """Byte-compile a single file."""
    try:
        py_compile.compile(fullname, ddir=ddir)
        return True
    except py_compile.PyCompileError:
        return False
