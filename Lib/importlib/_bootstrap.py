"""Minimal `importlib._bootstrap` stub.

CPython's real `_bootstrap` is the C-level import core. Only the handful
of names `pydoc.py` reads are provided; `_load(spec)` reloads/loads a
module by its spec's name through the public `import_module` path.
"""

import importlib


def _load(spec):
    name = spec.name
    if name in _loaded():
        return _loaded()[name]
    return importlib.import_module(name)


def _loaded():
    import sys
    return sys.modules


def _init_module_attrs(spec, module, *, override=False):
    return module


def _get_supported_file_loaders():
    return []


# `spec_from_loader`, `module_from_spec` etc. used by importlib internals.
def module_from_spec(spec):
    module = importlib.import_module(spec.name)
    return module
