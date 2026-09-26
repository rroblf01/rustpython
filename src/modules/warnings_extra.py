"""`warnings.deprecated` (PEP 702), installed onto the NATIVE `warnings`
module by `VirtualMachine::install_source_defined_stdlib` (see
`src/vm.rs`).

CPython 3.13 added this decorator and 3.14's own stdlib uses it widely —
`pdb.py` imports it at module scope, so without it `import pdb` raises
`ImportError: cannot import name 'deprecated' from 'warnings'`, which took
down everything that imports `pdb` (`bdb`-based tracebacks, `test_pdb`,
`test_debuglib`, the REPL's post-mortem path).

The native `warnings` module deliberately stays native (it owns the real,
shared filter list that `test.support` asserts against), so this one piece
of the 3.13+ surface is layered on top of it as Python source, exactly the
way `ssl.SSLSocket` or `enum.Enum` are layered onto their native halves.
"""

import functools
import warnings as _warnings


def deprecated(message, /, *, category=DeprecationWarning, stacklevel=1):
    """Indicate that a function, method or class is deprecated.

    Issue a `DeprecationWarning` when the thing is used, and record the
    message on it as `__deprecated__` so `warnings.deprecated`-aware
    tooling (and `-W error::DeprecationWarning` users) can see it.
    """
    if not isinstance(message, str):
        raise TypeError(
            f"Expected an object of type str for 'message', not "
            f"{type(message).__name__!r}"
        )

    def decorator(arg):
        msg = message
        if category is None:
            arg.__deprecated__ = msg
            return arg
        # Only a DeprecationWarning raised from library-internal code is
        # hidden by the default filter, so PEP 702 promotes those to
        # FutureWarning.
        cat = category
        if cat is DeprecationWarning and stacklevel < 2:
            cat = FutureWarning
        else:
            cat = category

        if isinstance(arg, type):
            original_new = arg.__new__

            @functools.wraps(original_new)
            def __new__(cls, /, *args, **kwargs):
                if cls is arg:
                    if cat is not None:
                        _warnings.warn(msg, category=cat, stacklevel=stacklevel + 1)
                if original_new is not object.__new__:
                    return original_new(cls, *args, **kwargs)
                # Mirrors CPython: object.__new__ only complains when extra
                # arguments are passed AND __init__ is not overridden.
                if cls is arg and (args or kwargs):
                    raise TypeError(
                        f"{cls.__name__}() takes no arguments "
                        f"({len(args) + len(kwargs)} given)"
                    )
                return original_new(cls)

            arg.__new__ = staticmethod(__new__)

            original_init_subclass = arg.__init_subclass__
            # We need slightly different behavior if __init_subclass__ is a
            # bound method (likely if it was implemented in Python)
            if isinstance(original_init_subclass, type.__init_subclass__):
                original_init_subclass = arg.__init_subclass__.__func__

                @functools.wraps(original_init_subclass)
                def __init_subclass__(*args, **kwargs):
                    if cat is not None:
                        _warnings.warn(msg, category=cat, stacklevel=stacklevel + 1)
                    return original_init_subclass(*args, **kwargs)

                arg.__init_subclass__ = classmethod(__init_subclass__)
            # Or otherwise, which likely means it's a builtin such as
            # object's implementation of __init_subclass__.
            else:
                @functools.wraps(original_init_subclass)
                def __init_subclass__(*args, **kwargs):
                    if cat is not None:
                        _warnings.warn(msg, category=cat, stacklevel=stacklevel + 1)
                    return original_init_subclass(*args, **kwargs)

                arg.__init_subclass__ = __init_subclass__

            arg.__deprecated__ = __new__.__deprecated__ = msg
            return arg
        elif callable(arg):
            import inspect

            @functools.wraps(arg)
            def wrapper(*args, **kwargs):
                if cat is not None:
                    _warnings.warn(msg, category=cat, stacklevel=stacklevel + 1)
                return arg(*args, **kwargs)

            if inspect.iscoroutinefunction(arg):
                wrapper = inspect.markcoroutinefunction(wrapper)

            arg.__deprecated__ = wrapper.__deprecated__ = msg
            return wrapper
        else:
            raise TypeError(
                "@deprecated decorator with non-None category must be "
                f"applied to a class or callable, not {type(arg).__name__!r}"
            )

    return decorator
