"""Minimal _bootstrap_external stub for RustPython.
Provides the bare minimum needed by importlib.machinery, pkgutil, and django."""

import _imp
import _io
import sys
import _warnings
import marshal
import posix as _os

_bootstrap = None

# Minimal wrappers for _check_name so machinary imports work
def _check_name(method):
    """Minimal check_name wrapper that skips name validation."""
    def _check_name_wrapper(self, name=None, *args, **kwargs):
        if name is None:
            name = self.name
        return method(self, name, *args, **kwargs)
    _check_name_wrapper.__name__ = method.__name__
    _check_name_wrapper.__qualname__ = method.__qualname__
    return _check_name_wrapper

# Cache from the importlib bootstrap
_module_type = type(sys)

# --- Constants ---
OPTIMIZED_BYTECODE_SUFFIXES = ['.pyc', '.pyo']
DEBUG_BYTECODE_SUFFIXES = ['.pyc']
BYTECODE_SUFFIXES = OPTIMIZED_BYTECODE_SUFFIXES
EXTENSION_SUFFIXES = _imp.extension_suffixes()
SOURCE_SUFFIXES = ['.py'] if hasattr(sys, 'dont_write_bytecode') else ['.py']
if _os.name == 'nt':
    EXTENSION_SUFFIXES += ['.pyd']
_magic = b'\x61\x0d\x0d\x0a'
MAGIC_NUMBER = (3571).to_bytes(2, 'little') + b'\r\n'
_RAW_MAGIC_NUMBER = MAGIC_NUMBER

def _pack_uint32(x):
    """Convert a 32-bit integer to little-endian bytes."""
    if x < 0:
        x = x & 0xFFFFFFFF
    return bytes([x & 0xFF, (x >> 8) & 0xFF, (x >> 16) & 0xFF, (x >> 24) & 0xFF])

def _unpack_uint32(data):
    """Convert little-endian bytes to a 32-bit integer."""
    return data[0] | (data[1] << 8) | (data[2] << 16) | (data[3] << 24)

def _classify_pyc(data, name, exc_details):
    """Minimal pyc validator."""
    if data[:4] == MAGIC_NUMBER:
        flags = _unpack_uint32(data[4:8])
        if flags & 0b10:
            hash_source = data[8:16]
        return 0
    raise ImportError(f'bad magic number in {name}', **exc_details)

def _validate_bytecode_header(data, name, exc_details):
    """Minimal bytecode header validator."""
    return _classify_pyc(data, name, exc_details)

def _compile_bytecode(data, name=None, bytecode_path=None, source_path=None):
    """Return code object from bytecode data."""
    from marshal import loads
    code = loads(data)
    if isinstance(code, type(lambda: None).__code__):
        if source_path is not None:
            _imp._fix_co_filename(code, source_path)
        return code
    raise ImportError(f'Non-code object in {bytecode_path}', name=name, path=bytecode_path)

def _code_to_timestamp_pyc(code, mtime=0, source_size=0):
    """Create timestamp-based pyc data."""
    data = bytearray(MAGIC_NUMBER)
    data.extend(_pack_uint32(0))
    data.extend(_pack_uint32(mtime))
    data.extend(_pack_uint32(source_size))
    data.extend(marshal.dumps(code))
    return bytes(data)

def _code_to_hash_pyc(code, source_hash, checked=True):
    """Create hash-based pyc data."""
    data = bytearray(MAGIC_NUMBER)
    flags = 0b1 | checked << 1
    data.extend(_pack_uint32(flags))
    data.extend(source_hash)
    data.extend(marshal.dumps(code))
    return bytes(data)

# --- Loader Classes ---
class BuiltinImporter:
    """Meta path import for built-in modules."""
    _name = 'BuiltinImporter'
    
    @staticmethod
    def find_module(name, path=None):
        if _imp.is_builtin(name):
            return BuiltinImporter
        return None
    
    @staticmethod
    def find_spec(name, path=None, target=None):
        if _imp.is_builtin(name):
            return None  # Will be handled by built-in import
        return None
    
    @staticmethod
    def load_module(name):
        mod = sys.modules.get(name)
        if mod is None:
            mod = _imp.create_builtin(type('spec', (), {'name': name})())
            _imp.exec_builtin(mod)
            sys.modules[name] = mod
        return mod

class FrozenImporter:
    """Meta path import for frozen modules."""
    _name = 'FrozenImporter'
    
    @staticmethod
    def find_module(name, path=None):
        if _imp.is_frozen(name):
            return FrozenImporter
        return None
    
    @staticmethod
    def find_spec(name, path=None, target=None):
        if _imp.is_frozen(name):
            return None
        return None

class PathEntryFinder:
    """Finder base class."""
    pass

class FileFinder:
    """Finder for modules on sys.path."""
    _name = 'FileFinder'
    
    @staticmethod
    def path_hook(*loader_details):
        def hook(path):
            return FileFinder(path)
        return hook
    
    def __init__(self, path, *loader_details):
        self.path = path
        self.name = path
    
    def find_spec(self, name, target=None):
        return None
    
    def find_module(self, name):
        return None

class SourceFileLoader:
    """Loader for source files."""
    _name = 'SourceFileLoader'
    
    def __init__(self, name, path):
        self.name = name
        self.path = path
    
    def get_code(self, fullname):
        # Return None to trigger loading from source
        return None
    
    def get_source(self, fullname):
        try:
            with _io.FileIO(self.path, 'r') as f:
                return f.read().decode('utf-8')
        except Exception:
            return None
    
    def get_filename(self, fullname):
        return self.path
    
    def exec_module(self, module):
        pass

class SourcelessFileLoader:
    """Loader for bytecode files without source."""
    _name = 'SourcelessFileLoader'
    
    def __init__(self, name, path):
        self.name = name
        self.path = path
    
    def get_code(self, fullname):
        return None

class ExtensionFileLoader:
    """Loader for extension modules (.so, .pyd)."""
    _name = 'ExtensionFileLoader'
    
    def __init__(self, name, path):
        self.name = name
        self.path = path
    
    def get_filename(self, fullname):
        return self.path
    
    def exec_module(self, module):
        raise ImportError(f'Cannot load extension module {self.name}')

# --- PathFinder ---
def _get_supported_file_loaders():
    """Return list of supported file loaders."""
    return [
        (SourceFileLoader, SOURCE_SUFFIXES),
        (SourcelessFileLoader, BYTECODE_SUFFIXES),
        (ExtensionFileLoader, EXTENSION_SUFFIXES),
    ]

class PathFinder:
    """Meta path finder for sys.path and sys.path_hooks."""
    _name = 'PathFinder'
    
    @staticmethod
    def find_spec(name, path=None, target=None):
        return None
    
    @staticmethod
    def find_module(name, path=None):
        return None
    
    @staticmethod
    def _path_hooks(path):
        return FileFinder

# Other loader stubs
class NamespaceLoader:
    _name = 'NamespaceLoader'

class AppleFrameworkLoader:
    _name = 'AppleFrameworkLoader'

WindowsRegistryFinder = None

class _LoaderBasics:
    pass

class _PotentialModule:
    pass

# --- Exports for importlib.machinery ---
_all_loaders = [
    SourceFileLoader, SourcelessFileLoader, ExtensionFileLoader,
    NamespaceLoader, AppleFrameworkLoader,
]

# --- _set_bootstrap_module ---
def _set_bootstrap_module(bootstrap_module):
    global _bootstrap
    _bootstrap = bootstrap_module

def _install(bootstrap_module):
    """Install the path-based import components."""
    _set_bootstrap_module(bootstrap_module)
    supported_loaders = _get_supported_file_loaders()
    sys.path_hooks.extend([FileFinder.path_hook(*supported_loaders)])
    sys.meta_path.append(PathFinder)
