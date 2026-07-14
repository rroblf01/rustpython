# Minimal stub for importlib.machinery
# Imports from _bootstrap_external when available
try:
    from importlib._bootstrap_external import (
        SOURCE_SUFFIXES, DEBUG_BYTECODE_SUFFIXES, OPTIMIZED_BYTECODE_SUFFIXES,
        BYTECODE_SUFFIXES, EXTENSION_SUFFIXES, WindowsRegistryFinder,
        PathFinder, FileFinder, SourceFileLoader, SourcelessFileLoader,
        ExtensionFileLoader, AppleFrameworkLoader, NamespaceLoader,
        _get_supported_file_loaders
    )
except ImportError:
    # Fallback: define simple stubs
    SOURCE_SUFFIXES = ['.py']
    DEBUG_BYTECODE_SUFFIXES = []
    OPTIMIZED_BYTECODE_SUFFIXES = []
    BYTECODE_SUFFIXES = []
    EXTENSION_SUFFIXES = []
    WindowsRegistryFinder = None
    PathFinder = None
    FileFinder = None
    SourceFileLoader = None
    SourcelessFileLoader = None
    ExtensionFileLoader = None
    AppleFrameworkLoader = None
    NamespaceLoader = None
