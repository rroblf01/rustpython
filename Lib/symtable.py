"""Symbol table module."""


class SymbolTable:
    """Symbol table for code."""

    def __init__(self):
        self.symbols = {}

    def get_symbols(self):
        return list(self.symbols.values())

    def get_identifiers(self):
        return set(self.symbols.keys())


class Symbol:
    """Symbol table entry."""

    def __init__(self, name, is_global=False, is_local=False, is_free=False,
                 is_assigned=False, is_parameter=False):
        self.name = name
        self.is_global = is_global
        self.is_local = is_local
        self.is_free = is_free
        self.is_assigned = is_assigned
        self.is_parameter = is_parameter


def symtable(code, filename, compile_type):
    """Return symbol table for code."""
    return SymbolTable()
