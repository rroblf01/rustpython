"""Stub constants for CPython's _symtable C extension.

Real symbol-table analysis (symtable.symtable()) needs compiler
integration (not implemented); these flag constants let
Lib/symtable.py import so test_symtable runs (and fails on
functionality) instead of crashing at import.
Values match CPython 3.14.
"""
USE = 16
DEF_GLOBAL = 1
DEF_NONLOCAL = 8
DEF_LOCAL = 2
DEF_PARAM = 4
DEF_TYPE_PARAM = 1024
DEF_FREE_CLASS = 64
DEF_IMPORT = 128
DEF_BOUND = 134
DEF_ANNOT = 256
DEF_COMP_ITER = 512
DEF_COMP_CELL = 2048
SCOPE_OFF = 12
SCOPE_MASK = 15
FREE = 4
LOCAL = 1
GLOBAL_IMPLICIT = 3
GLOBAL_EXPLICIT = 2
CELL = 5
TYPE_FUNCTION = 0
TYPE_CLASS = 1
TYPE_MODULE = 2
TYPE_ANNOTATION = 3
TYPE_TYPE_ALIAS = 4
TYPE_TYPE_PARAMETERS = 5
TYPE_TYPE_VARIABLE = 6
