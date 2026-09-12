# Real CPython's `_thread` (the C extension module) exposes a `_local`
# class — a fast, TLS-backed thread-local storage type. This project's
# `_thread` module is otherwise entirely native (Rust) with no pure-Python
# augmentation before now, so `_thread._local` was missing outright
# (`AttributeError: 'module' object has no attribute '_local'`), breaking
# any code importing it directly (`test_threading_local.py`'s
# `ThreadLocalTest`, which specifically exercises `_thread._local` side by
# side with the pure-Python fallback `_threading_local.local` to make sure
# both give the same observable behavior).
#
# This is deliberately self-contained (only `import _thread` — never
# `threading`/`_threading_local`) to avoid any bootstrap-ordering risk: it
# is installed via `VirtualMachine::install_source_defined_stdlib` directly
# into the ALREADY-registered native `_thread` module's own dict, using
# only names natively defined on that same module (`get_ident`,
# `allocate_lock`). The per-thread dict-swap logic mirrors real CPython's
# own pure-Python `_threading_local.local` (the documented, spec-following
# fallback for when the C `_thread._local` isn't available) rather than
# reinventing the semantics.
import _thread


class _local_impl:
    __slots__ = 'localargs', 'locallock', 'dicts'

    def __init__(self):
        self.dicts = {}


def _local_patch(self):
    impl = object.__getattribute__(self, '_local__impl')
    ident = _thread.get_ident()
    with impl.locallock:
        if ident in impl.dicts:
            dct = impl.dicts[ident]
        else:
            dct = {}
            impl.dicts[ident] = dct
            args, kw = impl.localargs
            # Re-run this subclass's own __init__ (if any) the first time
            # this particular logical thread touches the instance — matches
            # `_threading_local.local`'s documented per-thread lazy-init
            # behavior (e.g. `test_derived`'s `Local.__init__` needing to
            # run again for every new thread that accesses the instance).
            object.__setattr__(self, '__dict__', dct)
            type(self).__init__(self, *args, **kw)
        object.__setattr__(self, '__dict__', dct)


class _local:
    __slots__ = '_local__impl', '__dict__'

    def __new__(cls, /, *args, **kw):
        if (args or kw) and (cls.__init__ is object.__init__):
            raise TypeError("Initialization arguments are not supported")
        self = object.__new__(cls)
        impl = _local_impl()
        impl.localargs = (args, kw)
        impl.locallock = _thread.allocate_lock()
        object.__setattr__(self, '_local__impl', impl)
        impl.dicts[_thread.get_ident()] = {}
        object.__setattr__(self, '__dict__', impl.dicts[_thread.get_ident()])
        return self

    def __getattribute__(self, name):
        _local_patch(self)
        return object.__getattribute__(self, name)

    def __setattr__(self, name, value):
        if name == '__dict__':
            raise AttributeError(
                "%r object attribute '__dict__' is read-only"
                % self.__class__.__name__)
        _local_patch(self)
        return object.__setattr__(self, name, value)

    def __delattr__(self, name):
        if name == '__dict__':
            raise AttributeError(
                "%r object attribute '__dict__' is read-only"
                % self.__class__.__name__)
        _local_patch(self)
        return object.__delattr__(self, name)
