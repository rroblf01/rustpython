"""Local sysconfig package shim (test support).

The real `sysconfig` module is native; this package exists only so
`sysconfig.__main__` submodule imports resolve. Top-level `import
sysconfig` still hits the native module first (verified); this
__init__ is a fallback that re-exports nothing on its own.
"""
