# Minimal stub for importlib.util
from importlib.machinery import ModuleSpec

def spec_from_file_location(name, location=None, *, loader=None, submodule_search_locations=None):
    """Create a ModuleSpec from a file location."""
    if loader is None:
        from importlib.machinery import SourceFileLoader
        loader = SourceFileLoader(name, location)
    return ModuleSpec(name, loader, origin=location)

def module_from_spec(spec):
    """Create a new module from a ModuleSpec."""
    import types
    module = types.ModuleType(spec.name)
    if spec.origin:
        module.__file__ = spec.origin
    if spec.loader:
        module.__loader__ = spec.loader
    if spec.is_package:
        module.__path__ = []
    module.__spec__ = spec
    return module
