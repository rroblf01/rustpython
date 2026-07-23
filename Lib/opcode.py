"""Minimal `opcode` module stub.

Real CPython's `opcode.py` describes ITS OWN bytecode format (opcode
names/numbers, `stack_effect`, specialization metadata) — this interpreter
has a completely different bytecode format, so a real port doesn't apply.
This only exists so code that merely IMPORTS `opcode` (rather than asserting
on specific CPython opcode numbers) doesn't fail outright; introspection
helpers built on exact CPython opcode identity (`test.support.
bytecode_helper`) won't produce meaningful results here regardless.
"""

cmp_op = ('<', '<=', '==', '!=', '>', '>=')
hasarg = []
hasconst = []
hasname = []
hasjrel = []
hasjabs = []
haslocal = []
hascompare = []
hasfree = []
hasexc = []

opname = ['<%r>' % (i,) for i in range(256)]
opmap = {}
HAVE_ARGUMENT = 90
EXTENDED_ARG = 144


def stack_effect(*args, **kwargs):
    return 0
