"""Minimal pkgutil for RustPython — implements the subset real packages use.

Only iter_modules() and walk_packages() are provided, covering directory-based
packages (the common case: __path__ is a list of real filesystem directories).
Doesn't support zip imports, namespace-package merging, or custom finders.
"""

import os


class ModuleInfo:
    def __init__(self, module_finder, name, ispkg):
        self.module_finder = module_finder
        self.name = name
        self.ispkg = ispkg

    def __iter__(self):
        return iter((self.module_finder, self.name, self.ispkg))

    def __repr__(self):
        return "ModuleInfo(module_finder=%r, name=%r, ispkg=%r)" % (
            self.module_finder, self.name, self.ispkg,
        )


def iter_modules(path=None, prefix=""):
    if path is None:
        path = [os.getcwd()]
    seen = set()
    for entry in path:
        try:
            names = os.listdir(entry)
        except OSError:
            continue
        for name in sorted(names):
            if name.startswith("_") or name in seen:
                continue
            full = os.path.join(entry, name)
            if os.path.isdir(full):
                if os.path.isfile(os.path.join(full, "__init__.py")):
                    seen.add(name)
                    yield ModuleInfo(None, prefix + name, True)
            elif name.endswith(".py") and name != "__init__.py":
                mod_name = name[:-3]
                seen.add(mod_name)
                yield ModuleInfo(None, prefix + mod_name, False)


def walk_packages(path=None, prefix="", onerror=None):
    for info in iter_modules(path, prefix):
        yield info
        if info.ispkg:
            try:
                __import__(info.name)
                import sys
                sub_path = sys.modules[info.name].__path__
            except Exception as e:
                if onerror is not None:
                    onerror(info.name)
                continue
            yield from walk_packages(sub_path, info.name + ".", onerror)


def get_data(package, resource):
    import importlib
    mod = importlib.import_module(package)
    base = os.path.dirname(mod.__file__)
    path = os.path.join(base, resource)
    with open(path, "rb") as f:
        return f.read()
