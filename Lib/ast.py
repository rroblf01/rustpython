"""Pragmatic `ast` module.

Not a vendor of real CPython's `ast.py` (that one is generated from the C
`Python.asdl` grammar via `_ast`, a native extension this interpreter
doesn't have). This provides real, constructible node classes (enough for
code that BUILDS/inspects small ASTs by hand — real trigger: PEP 649 lazy
annotations in `annotationlib.py`'s `_Stringifier`, needed transitively by
`test.support`) plus a working `unparse` for the node shapes that code
actually produces. `ast.parse` is NOT a real parser bridge into this
interpreter's own parser/compiler (a separate, larger project) — it raises
`NotImplementedError`, which only matters for the (rare, deep) runtime path
of stringifying a lazy annotation that itself contains a nested string
expression to re-parse.
"""

from _ast_native import literal_eval

# compile() flag constants (same values as CPython's ast module).
PyCF_ONLY_AST = 0x40
PyCF_ALLOW_TOP_LEVEL_AWAIT = 0x8000
PyCF_TYPE_COMMENTS = 0x1000
PyCF_DONT_IMPLY_DEDENT = 0x200
PyCF_ACCEPT_NULL_BYTES = 0x10000000
PyCF_OPTIMIZED_AST = 0x1
PyCF_ASYNC_HINTS = 0x4000

__all__ = [
    "AST", "NodeVisitor", "NodeTransformer", "literal_eval", "parse", "unparse",
    "Module", "Expr", "Load", "Store", "Del",
    "BinOp", "UnaryOp", "Compare", "BoolOp", "Call", "Attribute", "Subscript",
    "Name", "Constant", "Dict", "List", "Set", "Tuple", "Slice", "Starred",
    "keyword", "Interpolation", "TemplateStr",
    "Add", "Sub", "Mult", "MatMult", "Div", "Mod", "LShift", "RShift",
    "BitOr", "BitXor", "BitAnd", "FloorDiv", "Pow",
    "UAdd", "USub", "Invert", "Not",
    "Eq", "NotEq", "Lt", "LtE", "Gt", "GtE", "Is", "IsNot", "In", "NotIn",
    "And", "Or",
]


class AST:
    """Base class. Subclasses declare `_fields` (a tuple of attribute
    names, in the same order real CPython's own grammar defines them) —
    positional constructor args fill those in order, keyword args set any
    field (overriding a positional value with the same name)."""

    _fields = ()

    def __init__(self, *args, **kwargs):
        if len(args) > len(self._fields):
            raise TypeError(f"{type(self).__name__} takes at most {len(self._fields)} positional arguments")
        for name, value in zip(self._fields, args):
            setattr(self, name, value)
        for name in self._fields[len(args):]:
            setattr(self, name, kwargs.pop(name, None))
        for name, value in kwargs.items():
            setattr(self, name, value)

    def __repr__(self):
        parts = ", ".join(f"{f}={getattr(self, f, None)!r}" for f in self._fields)
        return f"{type(self).__name__}({parts})"


def _node(name, fields=()):
    return type(name, (AST,), {"_fields": tuple(fields)})


# Statement / module wrappers
Module = _node("Module", ("body", "type_ignores"))
Expr = _node("Expr", ("value",))

# Expression contexts (markers, no fields)
Load = _node("Load")
Store = _node("Store")
Del = _node("Del")

# Binary operators (markers)
Add = _node("Add")
Sub = _node("Sub")
Mult = _node("Mult")
MatMult = _node("MatMult")
Div = _node("Div")
Mod = _node("Mod")
LShift = _node("LShift")
RShift = _node("RShift")
BitOr = _node("BitOr")
BitXor = _node("BitXor")
BitAnd = _node("BitAnd")
FloorDiv = _node("FloorDiv")
Pow = _node("Pow")

# Unary operators (markers)
UAdd = _node("UAdd")
USub = _node("USub")
Invert = _node("Invert")
Not = _node("Not")

# Comparison operators (markers)
Eq = _node("Eq")
NotEq = _node("NotEq")
Lt = _node("Lt")
LtE = _node("LtE")
Gt = _node("Gt")
GtE = _node("GtE")
Is = _node("Is")
IsNot = _node("IsNot")
In = _node("In")
NotIn = _node("NotIn")

# Boolean operators (markers)
And = _node("And")
Or = _node("Or")

# Expression nodes
BinOp = _node("BinOp", ("left", "op", "right"))
UnaryOp = _node("UnaryOp", ("op", "operand"))
BoolOp = _node("BoolOp", ("op", "values"))
Compare = _node("Compare", ("left", "ops", "comparators"))
Call = _node("Call", ("func", "args", "keywords"))
Attribute = _node("Attribute", ("value", "attr", "ctx"))
Subscript = _node("Subscript", ("value", "slice", "ctx"))
Name = _node("Name", ("id", "ctx"))
Constant = _node("Constant", ("value", "kind"))
Dict = _node("Dict", ("keys", "values"))
List = _node("List", ("elts", "ctx"))
Set = _node("Set", ("elts",))
Tuple = _node("Tuple", ("elts", "ctx"))
Slice = _node("Slice", ("lower", "upper", "step"))
Starred = _node("Starred", ("value", "ctx"))
keyword = _node("keyword", ("arg", "value"))
# PEP 750 t-strings — rare/deep, kept as plain field-bearing stand-ins.
Interpolation = _node("Interpolation", ("value", "expression", "conversion", "format_spec"))
TemplateStr = _node("TemplateStr", ("values",))


class NodeVisitor:
    def visit(self, node):
        method = "visit_" + type(node).__name__
        visitor = getattr(self, method, self.generic_visit)
        return visitor(node)

    def generic_visit(self, node):
        for field in getattr(node, "_fields", ()):
            value = getattr(node, field, None)
            if isinstance(value, list):
                for item in value:
                    if isinstance(item, AST):
                        self.visit(item)
            elif isinstance(value, AST):
                self.visit(value)
        return node


class NodeTransformer(NodeVisitor):
    def generic_visit(self, node):
        for field in getattr(node, "_fields", ()):
            value = getattr(node, field, None)
            if isinstance(value, list):
                new_values = []
                for item in value:
                    if isinstance(item, AST):
                        item = self.visit(item)
                        if item is None:
                            continue
                        if isinstance(item, list):
                            new_values.extend(item)
                            continue
                    new_values.append(item)
                setattr(node, field, new_values)
            elif isinstance(value, AST):
                new_value = self.visit(value)
                setattr(node, field, new_value)
        return node


def parse(source, filename="<unknown>", mode="exec"):
    raise NotImplementedError(
        "ast.parse() is not implemented in this interpreter — "
        "this ast module only supports building/inspecting nodes by hand"
    )


_BINOP_SYMBOLS = {
    Add: "+", Sub: "-", Mult: "*", MatMult: "@", Div: "/", Mod: "%",
    LShift: "<<", RShift: ">>", BitOr: "|", BitXor: "^", BitAnd: "&",
    FloorDiv: "//", Pow: "**",
}
_UNARYOP_SYMBOLS = {UAdd: "+", USub: "-", Invert: "~", Not: "not "}
_CMPOP_SYMBOLS = {
    Eq: "==", NotEq: "!=", Lt: "<", LtE: "<=", Gt: ">", GtE: ">=",
    Is: "is", IsNot: "is not", In: "in", NotIn: "not in",
}


def unparse(node):
    """Best-effort source reconstruction — covers the node shapes this
    module's own classes above can build, not the full real grammar."""
    if node is None:
        return ""
    if isinstance(node, Constant):
        return repr(node.value)
    if isinstance(node, Name):
        return node.id
    if isinstance(node, BinOp):
        sym = _BINOP_SYMBOLS.get(type(node.op), "?")
        return f"({unparse(node.left)} {sym} {unparse(node.right)})"
    if isinstance(node, UnaryOp):
        sym = _UNARYOP_SYMBOLS.get(type(node.op), "?")
        return f"({sym}{unparse(node.operand)})"
    if isinstance(node, BoolOp):
        sym = " and " if isinstance(node.op, And) else " or "
        return "(" + sym.join(unparse(v) for v in node.values) + ")"
    if isinstance(node, Compare):
        parts = [unparse(node.left)]
        for op, comp in zip(node.ops, node.comparators):
            parts.append(_CMPOP_SYMBOLS.get(type(op), "?"))
            parts.append(unparse(comp))
        return "(" + " ".join(parts) + ")"
    if isinstance(node, Call):
        args = [unparse(a) for a in node.args]
        args += [f"{kw.arg}={unparse(kw.value)}" if kw.arg else f"**{unparse(kw.value)}" for kw in node.keywords]
        return f"{unparse(node.func)}({', '.join(args)})"
    if isinstance(node, Attribute):
        return f"{unparse(node.value)}.{node.attr}"
    if isinstance(node, Subscript):
        return f"{unparse(node.value)}[{unparse(node.slice)}]"
    if isinstance(node, Slice):
        lower = unparse(node.lower) if node.lower is not None else ""
        upper = unparse(node.upper) if node.upper is not None else ""
        if node.step is not None:
            return f"{lower}:{upper}:{unparse(node.step)}"
        return f"{lower}:{upper}"
    if isinstance(node, Dict):
        pairs = ", ".join(f"{unparse(k)}: {unparse(v)}" for k, v in zip(node.keys, node.values))
        return "{" + pairs + "}"
    if isinstance(node, List):
        return "[" + ", ".join(unparse(e) for e in node.elts) + "]"
    if isinstance(node, Set):
        return "{" + ", ".join(unparse(e) for e in node.elts) + "}"
    if isinstance(node, Tuple):
        elts = ", ".join(unparse(e) for e in node.elts)
        return f"({elts},)" if len(node.elts) == 1 else f"({elts})"
    if isinstance(node, Starred):
        return f"*{unparse(node.value)}"
    if isinstance(node, keyword):
        return f"{node.arg}={unparse(node.value)}" if node.arg else f"**{unparse(node.value)}"
    return repr(node)
