# Minimal stub for importlib.machinery
# Self-contained — defines names directly to avoid import issues
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

class FileFinder:
    def __init__(self, path, *loader_details):
        self.path = path
    def find_spec(self, fullname, target=None):
        return None

class SourceFileLoader:
    def __init__(self, fullname, path):
        self.fullname = fullname
        self.path = path

class SourcelessFileLoader:
    def __init__(self, fullname, path):
        self.fullname = fullname
        self.path = path

class ExtensionFileLoader:
    def __init__(self, fullname, path):
        pass

class AppleFrameworkLoader:
    def __init__(self, fullname, path):
        pass

class NamespaceLoader:
    pass
