"""Pragmatic `ast` module.

Node classes mirror real CPython's own grammar (_fields copied from the
system interpreter's `ast` module — same names, same order) so code that
BUILDS or inspects ASTs by hand works (real trigger: PEP 649 lazy
annotations in `annotationlib.py`'s `_Stringifier`, needed transitively by
`test.support`). `ast.parse` bridges into this interpreter's own
parser/compiler (see `_ast_native.parse_source`, fed by the Rust-side
`Parser`) — it builds real node trees for the full statement/expression
grammar, with locations. `unparse` covers the node shapes produced both
by hand-built and parsed trees.
"""

from _ast_native import literal_eval, _parse_tree

# compile() flag constants (same values as CPython's ast module).
PyCF_ONLY_AST = 0x400
PyCF_ALLOW_TOP_LEVEL_AWAIT = 0x8000
PyCF_TYPE_COMMENTS = 0x1000
PyCF_DONT_IMPLY_DEDENT = 0x200
PyCF_ACCEPT_NULL_BYTES = 0x10000000
PyCF_OPTIMIZED_AST = 0x1
PyCF_ASYNC_HINTS = 0x4000

__all__ = [
    "AST", "NodeVisitor", "NodeTransformer", "literal_eval", "parse",
    "unparse", "dump", "walk", "fix_missing_locations", "copy_location",
    "increment_lineno", "get_source_segment",
    "Module", "Interactive", "Expression", "FunctionType",
    "FunctionDef", "AsyncFunctionDef", "ClassDef", "Return", "Delete",
    "Assign", "TypeAlias", "AugAssign", "AnnAssign", "For", "AsyncFor",
    "While", "If", "With", "AsyncWith", "Match", "Raise", "Try", "TryStar",
    "Assert", "Import", "ImportFrom", "Global", "Nonlocal", "Expr", "Pass",
    "Break", "Continue",
    "BoolOp", "NamedExpr", "BinOp", "UnaryOp", "Lambda", "IfExp", "Dict",
    "Set", "ListComp", "SetComp", "DictComp", "GeneratorExp", "Await",
    "Yield", "YieldFrom", "Compare", "Call", "JoinedStr", "FormattedValue",
    "Constant", "Attribute", "Subscript", "Starred", "Name", "List",
    "Tuple", "Slice",
    "Load", "Store", "Del", "AugLoad", "AugStore", "Param",
    "Add", "Sub", "Mult", "MatMult", "Div", "Mod", "LShift", "RShift",
    "BitOr", "BitXor", "BitAnd", "FloorDiv", "Pow",
    "UAdd", "USub", "Invert", "Not",
    "Eq", "NotEq", "Lt", "LtE", "Gt", "GtE", "Is", "IsNot", "In", "NotIn",
    "And", "Or",
    "ExceptHandler", "arguments", "arg", "keyword", "alias", "withitem",
    "match_case", "comprehension",
    "MatchValue", "MatchSingleton", "MatchSequence", "MatchMapping",
    "MatchClass", "MatchStar", "MatchAs", "MatchOr",
    "TypeIgnore", "TypeVar", "ParamSpec", "TypeVarTuple",
    "Slice", "Index", "ExtSlice", "Suite",
    "stmt", "expr", "expr_context", "operator", "boolop", "unaryop",
    "cmpop", "comprehension", "excepthandler", "pattern", "mod",
    "type_ignore", "type_param", "slice",
    "Interpolation", "TemplateStr",
]


class AST:
    """Base class. Subclasses declare `_fields` (a tuple of attribute
    names, in the same order real CPython's own grammar defines them) —
    positional constructor args fill those in order, keyword args set any
    field (overriding a positional value with the same name)."""

    _fields = ()
    _attributes = ("lineno", "col_offset", "end_lineno", "end_col_offset")

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
        return _repr_max_depth(self, 3)


def _repr_max_depth(node, depth):
    """Mirror CPython's C `ast_repr_max_depth` (Python-ast.c): full fields
    down to depth 0, `Type(...)` at/below it; lists show first+last
    (`[a, ..., z]` past 2 elements). No field omission — `kind=None`,
    `type_comment=None` all show."""
    if depth <= 0:
        return f"{type(node).__name__}(...)"
    if not isinstance(node, AST):
        return repr(node)
    parts = []
    for f in node._fields:
        v = getattr(node, f, None)
        if isinstance(v, (list, tuple)):
            parts.append(f"{f}={_repr_list(v, depth)}")
        elif isinstance(v, AST):
            parts.append(f"{f}={_repr_max_depth(v, depth - 1)}")
        else:
            parts.append(f"{f}={v!r}")
    return f"{type(node).__name__}({', '.join(parts)})"


def _repr_list(lst, depth):
    if len(lst) == 0:
        return repr(lst)
    is_list = isinstance(lst, list)
    out = []
    idxs = [0] if len(lst) == 1 else [0, len(lst) - 1]
    for i, idx in enumerate(idxs):
        if i > 0:
            out.append(", ")
        item = lst[idx]
        if isinstance(item, AST):
            out.append(_repr_max_depth(item, depth - 1))
        else:
            out.append(repr(item))
        if i == 0 and len(lst) > 2:
            out.append(", ...")
    bracket = ("[", "]") if is_list else ("(", ")")
    return bracket[0] + "".join(out) + bracket[1]


def _node(name, fields=()):
    return type(name, (AST,), {"_fields": tuple(fields)})


# Module / interactive / expression roots
Module = _node("Module", ("body", "type_ignores"))
Interactive = _node("Interactive", ("body",))
Expression = _node("Expression", ("body",))
FunctionType = _node("FunctionType", ("argtypes", "returns"))
Suite = _node("Suite", ("body",))

# Statement nodes
FunctionDef = _node("FunctionDef", ("name", "args", "body", "decorator_list", "returns", "type_comment", "type_params"))
AsyncFunctionDef = _node("AsyncFunctionDef", ("name", "args", "body", "decorator_list", "returns", "type_comment", "type_params"))
ClassDef = _node("ClassDef", ("name", "bases", "keywords", "body", "decorator_list", "type_params"))
Return = _node("Return", ("value",))
Delete = _node("Delete", ("targets",))
Assign = _node("Assign", ("targets", "value", "type_comment"))
TypeAlias = _node("TypeAlias", ("name", "type_params", "value"))
AugAssign = _node("AugAssign", ("target", "op", "value"))
AnnAssign = _node("AnnAssign", ("target", "annotation", "value", "simple"))
For = _node("For", ("target", "iter", "body", "orelse", "type_comment"))
AsyncFor = _node("AsyncFor", ("target", "iter", "body", "orelse", "type_comment"))
While = _node("While", ("test", "body", "orelse"))
If = _node("If", ("test", "body", "orelse"))
With = _node("With", ("items", "body", "type_comment"))
AsyncWith = _node("AsyncWith", ("items", "body", "type_comment"))
Match = _node("Match", ("subject", "cases"))
Raise = _node("Raise", ("exc", "cause"))
Try = _node("Try", ("body", "handlers", "orelse", "finalbody"))
TryStar = _node("TryStar", ("body", "handlers", "orelse", "finalbody"))
Assert = _node("Assert", ("test", "msg"))
Import = _node("Import", ("names",))
ImportFrom = _node("ImportFrom", ("module", "names", "level"))
Global = _node("Global", ("names",))
Nonlocal = _node("Nonlocal", ("names",))
Expr = _node("Expr", ("value",))
Pass = _node("Pass")
Break = _node("Break")
Continue = _node("Continue")

# Expression nodes
BoolOp = _node("BoolOp", ("op", "values"))
NamedExpr = _node("NamedExpr", ("target", "value"))
BinOp = _node("BinOp", ("left", "op", "right"))
UnaryOp = _node("UnaryOp", ("op", "operand"))
Lambda = _node("Lambda", ("args", "body"))
IfExp = _node("IfExp", ("test", "body", "orelse"))
Dict = _node("Dict", ("keys", "values"))
Set = _node("Set", ("elts",))
ListComp = _node("ListComp", ("elt", "generators"))
SetComp = _node("SetComp", ("elt", "generators"))
DictComp = _node("DictComp", ("key", "value", "generators"))
GeneratorExp = _node("GeneratorExp", ("elt", "generators"))
Await = _node("Await", ("value",))
Yield = _node("Yield", ("value",))
YieldFrom = _node("YieldFrom", ("value",))
Compare = _node("Compare", ("left", "ops", "comparators"))
Call = _node("Call", ("func", "args", "keywords"))
JoinedStr = _node("JoinedStr", ("values",))
FormattedValue = _node("FormattedValue", ("value", "conversion", "format_spec"))
Constant = _node("Constant", ("value", "kind"))
Attribute = _node("Attribute", ("value", "attr", "ctx"))
Subscript = _node("Subscript", ("value", "slice", "ctx"))
Starred = _node("Starred", ("value", "ctx"))
Name = _node("Name", ("id", "ctx"))
List = _node("List", ("elts", "ctx"))
Tuple = _node("Tuple", ("elts", "ctx"))
Slice = _node("Slice", ("lower", "upper", "step"))

# Expression contexts (markers, no fields)
Load = _node("Load")
Store = _node("Store")
Del = _node("Del")
AugLoad = _node("AugLoad")
AugStore = _node("AugStore")
Param = _node("Param")

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

# Compound helpers
ExceptHandler = _node("ExceptHandler", ("type", "name", "body"))
arguments = _node("arguments", ("posonlyargs", "args", "vararg", "kwonlyargs", "kw_defaults", "kwarg", "defaults"))
arg = _node("arg", ("arg", "annotation", "type_comment"))
keyword = _node("keyword", ("arg", "value"))
alias = _node("alias", ("name", "asname"))
withitem = _node("withitem", ("context_expr", "optional_vars"))
match_case = _node("match_case", ("pattern", "guard", "body"))
comprehension = _node("comprehension", ("target", "iter", "ifs", "is_async"))

# Match patterns
MatchValue = _node("MatchValue", ("value",))
MatchSingleton = _node("MatchSingleton", ("value",))
MatchSequence = _node("MatchSequence", ("patterns",))
MatchMapping = _node("MatchMapping", ("keys", "patterns", "rest"))
MatchClass = _node("MatchClass", ("cls", "patterns", "kwd_attrs", "kwd_patterns"))
MatchStar = _node("MatchStar", ("name",))
MatchAs = _node("MatchAs", ("pattern", "name"))
MatchOr = _node("MatchOr", ("patterns",))

# Type system
TypeIgnore = _node("TypeIgnore", ("lineno", "tag"))
TypeVar = _node("TypeVar", ("name", "bound", "default_value"))
ParamSpec = _node("ParamSpec", ("name", "default_value"))
TypeVarTuple = _node("TypeVarTuple", ("name", "default_value"))

# Legacy / abstract base markers
Index = _node("Index", ("value",))
ExtSlice = _node("ExtSlice", ("dims",))
stmt = _node("stmt")
expr = _node("expr")
expr_context = _node("expr_context")
operator = _node("operator")
boolop = _node("boolop")
unaryop = _node("unaryop")
cmpop = _node("cmpop")
excepthandler = _node("excepthandler")
pattern = _node("pattern")
mod = _node("mod")
type_ignore = _node("type_ignore")
type_param = _node("type_param")
slice = _node("slice")

# PEP 750 t-strings — rare/deep, kept as plain field-bearing stand-ins.
Interpolation = _node("Interpolation", ("value", "str", "conversion", "format_spec"))
TemplateStr = _node("TemplateStr", ("values",))

# Class-level None defaults for optional fields (mirrors CPython: `dump`
# omits a None field iff the class default is also None; fields without a
# class default always show, even when None).
_NODE_DEFAULTS = {
    "AnnAssign": ("value",),
    "Assert": ("msg",),
    "Assign": ("type_comment",),
    "AsyncFor": ("type_comment",),
    "AsyncFunctionDef": ("returns", "type_comment"),
    "AsyncWith": ("type_comment",),
    "Constant": ("kind",),
    "ExceptHandler": ("type", "name"),
    "For": ("type_comment",),
    "FormattedValue": ("format_spec",),
    "FunctionDef": ("returns", "type_comment"),
    "ImportFrom": ("module", "level"),
    "Interpolation": ("format_spec",),
    "MatchAs": ("pattern", "name"),
    "MatchMapping": ("rest",),
    "MatchStar": ("name",),
    "ParamSpec": ("default_value",),
    "Raise": ("exc", "cause"),
    "Return": ("value",),
    "Slice": ("lower", "upper", "step"),
    "TypeVar": ("bound", "default_value"),
    "TypeVarTuple": ("default_value",),
    "With": ("type_comment",),
    "Yield": ("value",),
    "alias": ("asname",),
    "arg": ("annotation", "type_comment"),
    "arguments": ("vararg", "kwarg"),
    "keyword": ("arg",),
    "match_case": ("guard",),
    "withitem": ("optional_vars",),
}
for _cls_name, _def_fields in _NODE_DEFAULTS.items():
    _cls = globals()[_cls_name]
    for _f in _def_fields:
        setattr(_cls, _f, None)
del _cls_name, _def_fields, _cls, _f


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


def parse(source, filename="<unknown>", mode="exec", **kwargs):
    """Parse source into an AST, using this interpreter's own parser.
    Accepts (and ignores, where unsupported) CPython's extra flags:
    `type_comments`, `feature_version`, `optimize`. An AST node input is
    returned as-is (mirrors `compile(tree, ...)` accepting trees)."""
    if isinstance(source, AST):
        return source
    return _parse_tree(source, filename, mode)


def dump(node, annotate_fields=True, include_attributes=False, indent=None,
         show_empty=False):
    """Return a formatted dump of the tree in node (mirrors CPython)."""
    def _format(node, level=0):
        if not isinstance(node, AST):
            return repr(node)
        cls = type(node)
        fields = []
        for name in node._fields:
            try:
                value = getattr(node, name)
            except AttributeError:
                continue
            # Omit None fields whose class default is also None (mirrors
            # CPython: `Constant.kind` hides, `Constant.value` shows).
            if value is None and getattr(cls, name, ...) is None:
                continue
            if not show_empty and value == []:
                continue
            if isinstance(value, AST):
                v = _format(value, level + 1)
            elif isinstance(value, list):
                v = "[" + ", ".join(_format(x, level + 1) for x in value) + "]"
            else:
                v = repr(value)
            if annotate_fields:
                fields.append(f"{name}={v}")
            else:
                fields.append(v)
        if include_attributes:
            for attr in ("lineno", "col_offset", "end_lineno", "end_col_offset"):
                if hasattr(node, attr):
                    fields.append(f"{attr}={getattr(node, attr)!r}")
        pad = "  " * level if indent is not None else ""
        inner = (",\n" + pad + "  " if indent is not None else ", ").join(fields)
        if indent is not None and fields:
            return f"{type(node).__name__}(\n{pad}  {inner}\n{pad})"
        return f"{type(node).__name__}({inner})"
    return _format(node)


def walk(node):
    """Recursively yield all descendant nodes in the tree (BFS)."""
    from collections import deque
    todo = deque([node])
    while todo:
        node = todo.popleft()
        todo.extend(_iter_child_nodes(node))
        yield node


def _iter_child_nodes(node):
    for field in getattr(node, "_fields", ()):
        try:
            value = getattr(node, field)
        except AttributeError:
            continue
        if isinstance(value, AST):
            yield value
        elif isinstance(value, list):
            for item in value:
                if isinstance(item, AST):
                    yield item


def _iter_fields(node):
    for field in getattr(node, "_fields", ()):
        try:
            yield field, getattr(node, field)
        except AttributeError:
            pass


def fix_missing_locations(node):
    """Fill lineno/col_offset on nodes missing them (mirrors CPython)."""
    for child in walk(node):
        if isinstance(child, AST):
            if not hasattr(child, "lineno"):
                child.lineno = getattr(node, "lineno", 1)
            if not hasattr(child, "col_offset"):
                child.col_offset = 0
            if not hasattr(child, "end_lineno"):
                child.end_lineno = getattr(child, "lineno", 1)
            if not hasattr(child, "end_col_offset"):
                child.end_col_offset = getattr(child, "col_offset", 0)
    return node


def copy_location(new_node, old_node):
    """Copy source location from old_node to new_node."""
    for attr in ("lineno", "col_offset", "end_lineno", "end_col_offset"):
        if hasattr(old_node, attr):
            setattr(new_node, attr, getattr(old_node, attr))
    return new_node


def increment_lineno(node, n=1):
    """Increment lineno of all nodes by n."""
    for child in walk(node):
        if hasattr(child, "lineno"):
            child.lineno = child.lineno + n
        if hasattr(child, "end_lineno"):
            child.end_lineno = child.end_lineno + n
    return node


def get_source_segment(source, node, *, padded=False):
    """Get source segment for node (mirrors CPython)."""
    if not hasattr(node, "lineno") or not hasattr(node, "end_lineno"):
        return None
    try:
        lines = source.splitlines(keepends=True)
        lineno = node.lineno - 1
        end_lineno = node.end_lineno - 1
        if lineno >= len(lines) or end_lineno >= len(lines):
            return None
        if lineno == end_lineno:
            seg = lines[lineno][node.col_offset:node.end_col_offset]
        else:
            parts = [lines[lineno][node.col_offset:]]
            parts.extend(lines[lineno + 1:end_lineno])
            parts.append(lines[end_lineno][:node.end_col_offset])
            seg = "".join(parts)
        if padded:
            return seg
        return seg.strip() if seg else None
    except (IndexError, AttributeError):
        return None


_BINOP_SYMBOLS = {
    Add: "+", Sub: "-", Mult: "*", MatMult: "@", Div: "/", Mod: "%",
    LShift: "<<", RShift: ">>", BitOr: "|", BitXor: "^", BitAnd: "&",
    FloorDiv: "//", Pow: "**",
}
_UNARYOP_SYMBOLS = {UAdd: "+", USub: "-", Invert: "~", Not: "not"}
_CMPOP_SYMBOLS = {
    Eq: "==", NotEq: "!=", Lt: "<", LtE: "<=", Gt: ">", GtE: ">=",
    Is: "is", IsNot: "is not", In: "in", NotIn: "not in",
}


# Precedence levels (mirrors CPython `_ast_unparse._Precedence`, auto() from 1).
_P_NAMED_EXPR = 1
_P_TUPLE = 2
_P_YIELD = 3
_P_TEST = 4
_P_OR = 5
_P_AND = 6
_P_NOT = 7
_P_CMP = 8
_P_EXPR = 9
_P_BOR = 9
_P_BXOR = 10
_P_BAND = 11
_P_SHIFT = 12
_P_ARITH = 13
_P_TERM = 14
_P_FACTOR = 15
_P_POWER = 16
_P_AWAIT = 17
_P_ATOM = 18

_BINOP_LEVELS = {
    BitOr: _P_BOR, BitXor: _P_BXOR, BitAnd: _P_BAND,
    LShift: _P_SHIFT, RShift: _P_SHIFT,
    Add: _P_ARITH, Sub: _P_ARITH,
    Mult: _P_TERM, MatMult: _P_TERM, Div: _P_TERM, Mod: _P_TERM,
    FloorDiv: _P_TERM, Pow: _P_POWER,
}
_UNARYOP_LEVELS = {Invert: _P_FACTOR, UAdd: _P_FACTOR, USub: _P_FACTOR, Not: _P_NOT}


def _unparse_ctx(ctx):
    return ""


def _par(s, need):
    return f"({s})" if need else s


def unparse(node, _prec=_P_TEST):
    """Source reconstruction with CPython-compatible minimal parentheses:
    `_prec` is the precedence the parent context requires; parens wrap
    iff required exceeds the node's own level."""
    if node is None:
        return ""
    if isinstance(node, Constant):
        if node.value is Ellipsis:
            return "..."
        if isinstance(node.value, (float, complex)):
            # Overflow to inf / nan must roundtrip as literals
            # (mirrors CPython `_write_constant`: inf -> 1e309-ish).
            r = repr(node.value)
            if "inf" in r or "nan" in r:
                import sys
                infstr = "1e" + repr(sys.float_info.max_10_exp + 1)
                r = r.replace("inf", infstr).replace("nan", f"({infstr}-{infstr})")
            return r
        return repr(node.value)
    if isinstance(node, Name):
        return node.id
    if isinstance(node, Attribute):
        v = unparse(node.value, _P_ATOM)
        # `3.attr` would lex as float `3.` + name (mirrors CPython's
        # space: `3 .__abs__()`).
        if isinstance(node.value, Constant) and isinstance(node.value.value, int):
            return f"{v} .{node.attr}"
        return f"{v}.{node.attr}"
    if isinstance(node, BinOp):
        level = _BINOP_LEVELS.get(type(node.op), _P_EXPR)
        sym = _BINOP_SYMBOLS.get(type(node.op), "?")
        if isinstance(node.op, Pow):
            left = unparse(node.left, level + 1)
            right = unparse(node.right, level)
        else:
            left = unparse(node.left, level)
            right = unparse(node.right, level + 1)
        return _par(f"{left} {sym} {right}", _prec > level)
    if isinstance(node, UnaryOp):
        level = _UNARYOP_LEVELS.get(type(node.op), _P_FACTOR)
        sym = _UNARYOP_SYMBOLS.get(type(node.op), "?")
        sep = "" if level == _P_FACTOR else " "
        return _par(f"{sym}{sep}{unparse(node.operand, level)}", _prec > level)
    if isinstance(node, BoolOp):
        level = _P_AND if isinstance(node.op, And) else _P_OR
        sym = " and " if isinstance(node.op, And) else " or "
        # Increasing requirement per value (mirrors CPython's
        # increasing_level_traverse for correct flat-list nesting).
        parts = []
        req = level
        for v in node.values:
            req += 1
            parts.append(unparse(v, req))
        return _par(sym.join(parts), _prec > level)
    if isinstance(node, NamedExpr):
        s = f"{unparse(node.target, _P_ATOM)} := {unparse(node.value, _P_ATOM)}"
        return _par(s, _prec > _P_NAMED_EXPR)
    if isinstance(node, Compare):
        parts = [unparse(node.left, _P_CMP + 1)]
        for op, comp in zip(node.ops, node.comparators):
            parts.append(_CMPOP_SYMBOLS.get(type(op), "?"))
            parts.append(unparse(comp, _P_CMP + 1))
        return _par(" ".join(parts), _prec > _P_CMP)
    if isinstance(node, Call):
        args = [unparse(a) for a in node.args]
        args += [f"{kw.arg}={unparse(kw.value)}" if kw.arg else f"**{unparse(kw.value)}" for kw in node.keywords]
        return f"{unparse(node.func, _P_ATOM)}({', '.join(args)})"
    if isinstance(node, Subscript):
        sl = node.slice
        # Non-empty tuple slices render bare (parens omitted):
        # `a[i, j]`, `a[i:j, k]` — never `a[(i, j)]`. A single element
        # keeps its trailing comma (`a[i,]`, `a[*a,]`).
        if isinstance(sl, Tuple) and sl.elts:
            inner = ", ".join(unparse(e) for e in sl.elts)
            if len(sl.elts) == 1:
                inner += ","
            return f"{unparse(node.value, _P_ATOM)}[{inner}]"
        return f"{unparse(node.value, _P_ATOM)}[{unparse(sl)}]"
    if isinstance(node, Slice):
        lower = unparse(node.lower) if node.lower is not None else ""
        upper = unparse(node.upper) if node.upper is not None else ""
        if node.step is not None:
            return f"{lower}:{upper}:{unparse(node.step)}"
        return f"{lower}:{upper}"
    if isinstance(node, Dict):
        pairs = []
        for k, v in zip(node.keys, node.values):
            if k is None:
                pairs.append(f"**{unparse(v, _P_EXPR)}")
            else:
                pairs.append(f"{unparse(k)}: {unparse(v)}")
        return "{" + ", ".join(pairs) + "}"
    if isinstance(node, List):
        return "[" + ", ".join(unparse(e) for e in node.elts) + "]"
    if isinstance(node, Set):
        if not node.elts:
            return "{*()}"
        return "{" + ", ".join(unparse(e) for e in node.elts) + "}"
    if isinstance(node, Tuple):
        if not node.elts:
            return "()"
        elts = ", ".join(unparse(e) for e in node.elts)
        if len(node.elts) == 1:
            elts += ","
        # Parens iff required (targets render bare: `x, y = ...`).
        if not node.elts or _prec > _P_TUPLE:
            return f"({elts})"
        return elts
    if isinstance(node, Starred):
        return f"*{unparse(node.value, _P_EXPR)}"
    if isinstance(node, keyword):
        return f"{node.arg}={unparse(node.value)}" if node.arg else f"**{unparse(node.value)}"
    if isinstance(node, ListComp):
        gens = " ".join(_unparse_comp(g) for g in node.generators)
        return f"[{unparse(node.elt)} {gens}]"
    if isinstance(node, SetComp):
        gens = " ".join(_unparse_comp(g) for g in node.generators)
        return "{" + f"{unparse(node.elt)} {gens}" + "}"
    if isinstance(node, DictComp):
        gens = " ".join(_unparse_comp(g) for g in node.generators)
        return "{" + f"{unparse(node.key)}: {unparse(node.value)} {gens}" + "}"
    if isinstance(node, GeneratorExp):
        gens = " ".join(_unparse_comp(g) for g in node.generators)
        return f"({unparse(node.elt)} {gens})"
    if isinstance(node, Await):
        s = f"await {unparse(node.value, _P_ATOM)}" if node.value is not None else "await"
        return _par(s, _prec > _P_AWAIT)
    if isinstance(node, Yield):
        s = f"yield {unparse(node.value, _P_ATOM)}" if node.value is not None else "yield"
        return _par(s, _prec > _P_YIELD)
    if isinstance(node, YieldFrom):
        if node.value is None:
            raise ValueError("YieldFrom requires a value")
        s = f"yield from {unparse(node.value, _P_ATOM)}"
        return _par(s, _prec > _P_YIELD)
    if isinstance(node, Lambda):
        if _has_args(node.args):
            s = f"lambda {unparse(node.args)}: {unparse(node.body, _P_TEST)}"
        else:
            s = f"lambda: {unparse(node.body, _P_TEST)}"
        return _par(s, _prec > _P_TEST)
    if isinstance(node, IfExp):
        s = f"{unparse(node.body, _P_TEST + 1)} if {unparse(node.test, _P_TEST + 1)} else {unparse(node.orelse, _P_TEST)}"
        return _par(s, _prec > _P_TEST)
    if isinstance(node, JoinedStr):
        return _unparse_fstring(node.values)
    if isinstance(node, FormattedValue):
        return "{" + _unparse_formatted_f(node) + "}"
    if isinstance(node, TemplateStr):
        return _unparse_fstring(node.values, prefix="t")
    if isinstance(node, Interpolation):
        return "{" + _unparse_formatted_f(node) + "}"
    if isinstance(node, arguments):
        return _unparse_arguments(node)
    if isinstance(node, FunctionType):
        args = ", ".join(unparse(a) for a in node.argtypes)
        return f"({args}) -> {unparse(node.returns)}"
    if isinstance(node, arg):
        return node.arg + (f": {unparse(node.annotation)}" if node.annotation else "")
    if isinstance(node, alias):
        return node.name + (f" as {node.asname}" if node.asname else "")
    # Statements
    if isinstance(node, Module):
        return "\n".join(_unparse_body(node.body, indent="", toplevel=True, docstring=True))
    if isinstance(node, Interactive):
        # Single-mode bodies join with "; " (gh-129598), not newlines.
        return "; ".join(unparse(s) for s in node.body)
    if isinstance(node, Expression):
        return unparse(node.body)
    if isinstance(node, Expr):
        return unparse(node.value, _P_YIELD)
    if isinstance(node, (FunctionDef, AsyncFunctionDef)):
        async_p = "async " if isinstance(node, AsyncFunctionDef) else ""
        decos = "".join(f"@{unparse(d)}\n" for d in node.decorator_list or [])
        ret = f" -> {unparse(node.returns)}" if node.returns else ""
        tparams = ""
        if node.type_params:
            tparams = "[" + ", ".join(unparse(t) for t in node.type_params) + "]"
        body = "\n".join(_unparse_body(node.body, docstring=True))
        return f"{decos}{async_p}def {node.name}{tparams}({unparse(node.args)}){ret}:\n{body}"
    if isinstance(node, ClassDef):
        decos = "".join(f"@{unparse(d)}\n" for d in node.decorator_list or [])
        bases = [unparse(b) for b in node.bases or []] + [unparse(k) for k in node.keywords or []]
        tparams = ""
        if node.type_params:
            tparams = "[" + ", ".join(unparse(t) for t in node.type_params) + "]"
        hdr = f"({', '.join(bases)})" if bases else ""
        body = "\n".join(_unparse_body(node.body, docstring=True))
        return f"{decos}class {node.name}{tparams}{hdr}:\n{body}"
    if isinstance(node, Return):
        return f"return {unparse(node.value)}" if node.value is not None else "return"
    if isinstance(node, Delete):
        return "del " + ", ".join(unparse(t) for t in node.targets)
    if isinstance(node, Assign):
        return " = ".join(unparse(t, _P_TUPLE) for t in node.targets) + f" = {unparse(node.value)}"
    if isinstance(node, AugAssign):
        sym = _BINOP_SYMBOLS.get(type(node.op), "?")
        return f"{unparse(node.target)} {sym}= {unparse(node.value)}"
    if isinstance(node, AnnAssign):
        s = f"{unparse(node.target)}: {unparse(node.annotation)}"
        if node.value is not None:
            s += f" = {unparse(node.value)}"
        return s
    if isinstance(node, (For, AsyncFor)):
        async_p = "async " if isinstance(node, AsyncFor) else ""
        s = f"{async_p}for {unparse(node.target, _P_TUPLE)} in {unparse(node.iter)}:\n"
        s += "\n".join(_unparse_body(node.body))
        if node.orelse:
            s += "\nelse:\n" + "\n".join(_unparse_body(node.orelse))
        return s
    if isinstance(node, While):
        s = f"while {unparse(node.test)}:\n"
        s += "\n".join(_unparse_body(node.body))
        if node.orelse:
            s += "\nelse:\n" + "\n".join(_unparse_body(node.orelse))
        return s
    if isinstance(node, If):
        s = f"if {unparse(node.test)}:\n"
        s += "\n".join(_unparse_body(node.body))
        if node.orelse:
            s += "\nelse:\n" + "\n".join(_unparse_body(node.orelse))
        return s
    if isinstance(node, (With, AsyncWith)):
        async_p = "async " if isinstance(node, AsyncWith) else ""
        items = ", ".join(_unparse_withitem(i) for i in node.items)
        body = "\n".join(_unparse_body(node.body))
        return f"{async_p}with {items}:\n{body}"
    if isinstance(node, Raise):
        if node.exc is None:
            if node.cause is not None:
                raise ValueError("raise with cause but no exception")
            return "raise"
        if node.cause is None:
            return f"raise {unparse(node.exc)}"
        return f"raise {unparse(node.exc)} from {unparse(node.cause)}"
    if isinstance(node, (Try, TryStar)):
        # The star lives on the handlers (`except*`), never on `try:`.
        estar = "*" if isinstance(node, TryStar) else ""
        s = "try:\n" + "\n".join(_unparse_body(node.body))
        for h in node.handlers:
            t = f"except{estar} {unparse(h.type)}" if h.type else f"except{estar}"
            if h.name:
                t += f" as {h.name}"
            s += f"\n{t}:\n" + "\n".join(_unparse_body(h.body))
        if node.orelse:
            s += "\nelse:\n" + "\n".join(_unparse_body(node.orelse))
        if node.finalbody:
            s += "\nfinally:\n" + "\n".join(_unparse_body(node.finalbody))
        return s
    if isinstance(node, Assert):
        if node.msg is not None:
            return f"assert {unparse(node.test)}, {unparse(node.msg)}"
        return f"assert {unparse(node.test)}"
    if isinstance(node, Import):
        return "import " + ", ".join(unparse(a) for a in node.names)
    if isinstance(node, ImportFrom):
        dots = "." * (node.level or 0)
        mod = node.module or ""
        return f"from {dots}{mod} import " + ", ".join(unparse(a) for a in node.names)
    if isinstance(node, Global):
        return "global " + ", ".join(node.names)
    if isinstance(node, Nonlocal):
        return "nonlocal " + ", ".join(node.names)
    if isinstance(node, Pass):
        return "pass"
    if isinstance(node, Break):
        return "break"
    if isinstance(node, Continue):
        return "continue"
    if isinstance(node, TypeAlias):
        params = ""
        if node.type_params:
            params = "[" + ", ".join(unparse(t) for t in node.type_params) + "]"
        return f"type {unparse(node.name)}{params} = {unparse(node.value)}"
    if isinstance(node, TypeVar):
        s = node.name
        if node.bound is not None:
            s += f": {unparse(node.bound)}"
        if node.default_value is not None:
            s += f" = {unparse(node.default_value)}"
        return s
    if isinstance(node, ParamSpec):
        # Renders with `**` in type-parameter lists (CPython:
        # `def f[**P = 1]`).
        s = f"**{node.name}"
        if node.default_value is not None:
            s += f" = {unparse(node.default_value)}"
        return s
    if isinstance(node, TypeVarTuple):
        s = f"*{node.name}"
        if node.default_value is not None:
            s += f" = {unparse(node.default_value)}"
        return s
    if isinstance(node, Match):
        s = f"match {unparse(node.subject)}:\n"
        for case in node.cases:
            s += "    case " + _unparse_pattern(case.pattern)
            if case.guard is not None:
                s += f" if {unparse(case.guard)}"
            s += ":\n"
            for b in case.body:
                s += "        " + unparse(b).replace("\n", "\n        ") + "\n"
        return s.rstrip("\n")
    return repr(node)


def _unparse_pattern(pat):
    if isinstance(pat, MatchValue):
        return unparse(pat.value)
    if isinstance(pat, MatchSingleton):
        if pat.value is True:
            return "True"
        if pat.value is False:
            return "False"
        if pat.value is None:
            return "None"
        return repr(pat.value)
    if isinstance(pat, MatchSequence):
        return "[" + ", ".join(_unparse_pattern(p) for p in pat.patterns) + "]"
    if isinstance(pat, MatchMapping):
        pairs = [f"{unparse(k)}: {_unparse_pattern(p)}" for k, p in zip(pat.keys, pat.patterns)]
        if pat.rest is not None:
            if pairs:
                pairs.append(f"**{pat.rest}")
            else:
                pairs = [f"**{pat.rest}"]
        return "{" + ", ".join(pairs) + "}"
    if isinstance(pat, MatchClass):
        parts = [_unparse_pattern(p) for p in pat.patterns]
        parts += [f"{a}={_unparse_pattern(p)}" for a, p in zip(pat.kwd_attrs, pat.kwd_patterns)]
        return f"{unparse(pat.cls, _P_ATOM)}({', '.join(parts)})"
    if isinstance(pat, MatchStar):
        return f"*{pat.name if pat.name is not None else '_'}"
    if isinstance(pat, MatchAs):
        if pat.name is None:
            return "_"
        if pat.pattern is None:
            return pat.name
        inner = _unparse_pattern(pat.pattern)
        # `as` binds looser than `|`: parenthesize or-alternatives.
        if isinstance(pat.pattern, MatchOr):
            inner = f"({inner})"
        return f"{inner} as {pat.name}"
    if isinstance(pat, MatchOr):
        return " | ".join(_unparse_pattern(p) for p in pat.patterns)
    return unparse(pat)


def _has_args(args):
    return bool(
        args.posonlyargs or args.args or args.vararg
        or args.kwonlyargs or args.kwarg
    )


def _unparse_comp(g):
    s = ("async for " if g.is_async else "for ") + unparse(g.target, _P_TUPLE) + f" in {unparse(g.iter)}"
    for cond in g.ifs:
        s += f" if {unparse(cond)}"
    return s


def _unparse_fstring(values, prefix="f"):
    """Render an f-string picking quotes that work for ALL parts (mirrors
    CPython `_ftstring_helper`): constants narrow via `_str_literal_helper`,
    nested sources exclude any quote they contain; fallback to `'''` with
    repr-escaped constants when disjoint."""
    rendered = []
    for v in values:
        if isinstance(v, Constant) and isinstance(v.value, str):
            # Literal braces must double (else they'd parse as fields).
            rendered.append((v.value.replace("{", "{{").replace("}", "}}"), True))
        elif isinstance(v, FormattedValue):
            rendered.append(("{" + _unparse_formatted_f(v) + "}", False))
        elif isinstance(v, Interpolation):
            rendered.append(("{" + _unparse_formatted_f(v) + "}", False))
        elif isinstance(v, TemplateStr):
            rendered.append((_unparse_fstring(v.values, prefix="t"), False))
        elif isinstance(v, JoinedStr):
            rendered.append((_unparse_fstring(v.values, prefix), False))
        else:
            # Only Constant/FormattedValue/Interpolation (and nested
            # JoinedStr/TemplateStr) are legal inside an f-string —
            # anything else (e.g. a bare Name) is invalid AST.
            raise ValueError(f"Unexpected node inside JoinedStr, {v!r}")
    quote_types = ["'", '"', '"""', "'''"]
    new_parts = []
    fallback = False
    for value, is_const in rendered:
        if is_const:
            value, new_q = _str_literal_helper(
                value, quote_types=quote_types, escape_special_whitespace=True
            )
            if set(new_q).isdisjoint(quote_types):
                fallback = True
                break
            quote_types = new_q
        else:
            if "\n" in value:
                quote_types = [q for q in quote_types if len(q) == 3]
                assert quote_types
            new_q = [q for q in quote_types if q not in value]
            if new_q:
                quote_types = new_q
        new_parts.append(value)
    if fallback:
        quote_types = ["'''"]
        new_parts = []
        for value, is_const in rendered:
            if is_const:
                r = repr('"' + value)
                assert r.startswith("'\""), repr(value)
                value = r[2:-1]
            new_parts.append(value)
    q = quote_types[0]
    return f"{prefix}{q}{''.join(new_parts)}{q}"


def _unparse_formatted_f(node):
    """FormattedValue source for fstring assembly (nested fstrings render
    recursively with their own quotes). Interpolations (PEP 750) render
    their raw `str` text when present, falling back to the value."""
    if isinstance(node, Interpolation) and isinstance(node.str, str):
        s = node.str
    else:
        v = node.value
        s = unparse(v)
        # Low-precedence values need parens inside `{...}` (mirrors
        # CPython: `f'{(lambda: x)}'`, never `f'{lambda: x}'`).
        if isinstance(v, (Lambda, IfExp, Yield, YieldFrom, Tuple, NamedExpr)):
            s = f"({s})"
        if s.startswith("{"):
            # A leading `{` would read as a `{{` literal-brace escape
            # on re-parse (mirrors CPython: `f'{ {1: 2}}'`).
            s = " " + s
    if node.conversion and node.conversion >= 0:
        s += "!" + ("r" if node.conversion == 114 else "s" if node.conversion == 115 else "a")
    if node.format_spec is not None:
        spec = node.format_spec
        if isinstance(spec, JoinedStr):
            inner = []
            for v in spec.values:
                if isinstance(v, Constant) and isinstance(v.value, str):
                    # Inside a format spec, quotes/backslashes/newlines
                    # escape with a backslash (mirrors CPython
                    # `_write_ftstring_inner(is_format_spec=True)`).
                    inner.append(
                        v.value.replace("\\", "\\\\")
                        .replace("'", "\\'")
                        .replace('"', '\\"')
                        .replace("\n", "\\n")
                    )
                elif isinstance(v, (FormattedValue, Interpolation)):
                    inner.append("{" + _unparse_formatted_f(v) + "}")
                elif isinstance(v, TemplateStr):
                    inner.append(_unparse_fstring(v.values, prefix="t"))
                elif isinstance(v, JoinedStr):
                    inner.append(_unparse_fstring(v.values))
                else:
                    inner.append("{" + unparse(v) + "}")
            s += ":" + "".join(inner)
        else:
            s += ":" + unparse(spec)
    return s


def _is_docstring(stmt):
    return (
        isinstance(stmt, Expr)
        and isinstance(stmt.value, Constant)
        and isinstance(stmt.value.value, str)
    )


_ESCAPES = {
    "\\": "\\\\", "\n": "\\n", "\r": "\\r", "\t": "\\t",
    "\b": "\\b", "\f": "\\f", "\v": "\\v", "\0": "\\0",
}


def _str_literal_helper(string, quote_types=("'", '"', '"""', "'''"),
                         escape_special_whitespace=False):
    """Mirror CPython `_ast_unparse._str_literal_helper`: pick quotes
    minimizing escapes; returns (escaped_body, [quotes])."""
    def escape_char(c):
        if not escape_special_whitespace and c in "\n\t":
            return c
        if c in _ESCAPES:
            return _ESCAPES[c]
        if not c.isprintable():
            code = ord(c)
            if code < 0x100:
                return f"\\x{code:02x}"
            if code < 0x10000:
                return f"\\u{code:04x}"
            return f"\\U{code:08x}"
        return c

    escaped = "".join(escape_char(c) for c in string)
    possible = list(quote_types)
    if "\n" in escaped:
        possible = [q for q in possible if len(q) == 3]
    possible = [q for q in possible if q not in escaped]
    if not possible:
        r = repr(string)
        quote = next((q for q in quote_types if r[0] in q), r[0])
        return r[1:-1], [quote]
    if escaped:
        possible.sort(key=lambda q: q[0] == escaped[-1])
        if possible[0][0] == escaped[-1]:
            assert len(possible[0]) == 3
            escaped = escaped[:-1] + "\\" + escaped[-1]
    return escaped, possible


def _unparse_docstring(value):
    # Triple quotes only (module/class/function docstring position).
    s, quotes = _str_literal_helper(value, quote_types=('"""', "'''"))
    return f"{quotes[0]}{s}{quotes[0]}"


def _unparse_body(body, indent="    ", toplevel=False, docstring=False):
    parts = []
    for i, s in enumerate(body):
        # Blank line before every def/class (CPython `maybe_newline` in
        # `_function_helper`/ClassDef): at top level only when not first;
        # in nested bodies always (even before the first statement).
        if isinstance(s, (FunctionDef, AsyncFunctionDef, ClassDef)) and (i > 0 or not toplevel):
            parts.append("")
        # Triple-quoted docstrings only in real docstring positions
        # (module/class/function bodies) — never in if/for/while bodies
        # (mirrors CPython: `if i: """e"""` unparses with single quotes).
        if i == 0 and docstring and _is_docstring(s):
            parts.append(indent + _unparse_docstring(s.value.value))
        else:
            parts.append(indent + unparse(s).replace("\n", "\n" + indent))
    return parts
    s = ("async for " if g.is_async else "for ") + unparse(g.target) + f" in {unparse(g.iter)}"
    for cond in g.ifs:
        s += f" if {unparse(cond)}"
    return s


def _unparse_withitem(item):
    if item.optional_vars is not None:
        return f"{unparse(item.context_expr)} as {unparse(item.optional_vars)}"
    return unparse(item.context_expr)


def _unparse_arguments(node):
    parts = []
    posonly = list(node.posonlyargs or [])
    args = list(node.args or [])
    all_args = posonly + args
    defaults = [None] * (len(all_args) - len(node.defaults or [])) + list(node.defaults or [])
    for i, (a, d) in enumerate(zip(all_args, defaults)):
        s = unparse(a)
        if d is not None:
            s += f"={unparse(d)}"
        parts.append(s)
        if i + 1 == len(posonly):
            parts.append("/")
    if node.vararg or node.kwonlyargs:
        if node.vararg:
            parts.append("*" + unparse(node.vararg))
        else:
            parts.append("*")
    for a, d in zip(node.kwonlyargs or [], node.kw_defaults or []):
        # kw_defaults pads with None for default-less kwonly args.
        parts.append(unparse(a) + (f"={unparse(d)}" if d is not None else ""))
    if node.kwarg:
        parts.append("**" + unparse(node.kwarg))
    return ", ".join(parts)
