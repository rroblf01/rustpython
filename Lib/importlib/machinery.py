# Minimal stub for importlib.machinery
# Self-contained — defines names directly to avoid import issues
import os
import sys

SOURCE_SUFFIXES = ['.py']
DEBUG_BYTECODE_SUFFIXES = []
OPTIMIZED_BYTECODE_SUFFIXES = []
BYTECODE_SUFFIXES = []
EXTENSION_SUFFIXES = []
_all_suffixes = SOURCE_SUFFIXES + DEBUG_BYTECODE_SUFFIXES + OPTIMIZED_BYTECODE_SUFFIXES

def _get_supported_file_loaders():
    return []

class WindowsRegistryFinder:
    @classmethod
    def find_spec(cls, fullname, path=None, target=None):
        return None

class PathFinder:
    @classmethod
    def find_spec(cls, fullname, path=None, target=None):
        return None
    @classmethod
    def invalidate_caches(cls):
        return None
    @classmethod
    def find_module(cls, fullname, path=None):
        return None

class FileFinder:
    def __init__(self, path, *loader_details):
        self.path = path
    def find_spec(self, fullname, target=None):
        return None

class _FileLoaderMixin:
    """The read side of CPython's `importlib._bootstrap_external.FileLoader`.

    Real code reaches for these constantly: `linecache.updatecache`/
    `lazycache` call `loader.get_source(name)` to recover a module's source
    when its file is not on disk (that is how `traceback`, `doctest` and
    `inspect.getsource` read a `__main__` whose `__file__` moved), and
    `importlib.reload`, `pkgutil` and `zipimport` use the rest.
    """

    def get_filename(self, fullname=None):
        return self.path

    def get_data(self, path):
        with open(path, 'rb') as f:
            return f.read()

    def path_stats(self, path):
        st = os.stat(path)
        return st.st_mtime, st.st_size

    def module_repr(self, module):
        return '<%s for %s>' % (type(self).__name__, self.path)

    def is_package(self, fullname=None):
        return False

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        code = self.get_code(module.__name__)
        if code is None:
            raise ImportError('cannot load %r' % self.path)
        exec(code, module.__dict__)

    def load_module(self, fullname):
        import importlib.util
        spec = importlib.util.spec_from_file_location(
            fullname, self.path, loader=self)
        if spec is None:
            raise ImportError('spec not found for %r' % self.path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[fullname] = module
        try:
            self.exec_module(module)
        except BaseException:
            del sys.modules[fullname]
            raise
        return sys.modules[fullname]


class SourceFileLoader(_FileLoaderMixin):
    def __init__(self, fullname, path):
        self.fullname = fullname
        self.name = fullname
        self.path = path

    def get_source(self, fullname=None):
        # `tokenize.open` is CPython's own PEP 263-aware reader: it honours
        # the coding declaration and RAISES for a broken one, which is what
        # makes `get_source` return None (a caller then treats the module as
        # having no source) rather than return mis-decoded text.
        import tokenize
        try:
            with tokenize.open(self.path) as f:
                return f.read()
        except (OSError, UnicodeDecodeError, SyntaxError, ValueError):
            return None

    def get_code(self, fullname=None):
        source = self.get_source(fullname or self.fullname)
        if source is None:
            return None
        return compile(source, self.path, 'exec',
                       dont_inherit=True, optimize=-1)


class SourcelessFileLoader(_FileLoaderMixin):
    def __init__(self, fullname, path):
        self.fullname = fullname
        self.name = fullname
        self.path = path

    def get_source(self, fullname=None):
        return None

    def get_code(self, fullname=None):
        import marshal
        with open(self.path, 'rb') as f:
            data = f.read()
        return marshal.loads(data[16:])


class ExtensionFileLoader:
    def __init__(self, fullname, path):
        pass

class AppleFrameworkLoader:
    def __init__(self, fullname, path):
        pass

class NamespaceLoader:
    pass

class ModuleSpec:
    """A module specification (PEP 451)."""
    def __init__(self, name, loader, *, origin=None, loader_state=None, is_package=False):
        self.name = name
        self.loader = loader
        self.origin = origin
        self.loader_state = loader_state
        self.is_package = is_package
        self.parent = None
        self.submodule_search_locations = None
    def __repr__(self):
        return f"ModuleSpec(name={self.name!r}, loader={self.loader!r})"
    def __eq__(self, other):
        if not isinstance(other, ModuleSpec):
            return NotImplemented
        return self.name == other.name and self.loader == other.loader
    def __hash__(self):
        return hash((self.name, self.loader))
