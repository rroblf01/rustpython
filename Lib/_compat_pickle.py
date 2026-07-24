"""Compatibility module for pickle between Python versions."""

# Mapping of Python 2 to Python 3 names for pickle compatibility

IMPORT_MAPPING = {
    'copy_reg': 'copyreg',
    'Queue': 'queue',
    '__builtin__': 'builtins',
    'exceptions': 'builtins',
    'ConfigParser': 'configparser',
    'StringIO': 'io',
    'cStringIO': 'io',
    'thread': '_thread',
    'dummy_thread': '_dummy_thread',
}

REVERSE_IMPORT_MAPPING = {v: k for k, v in IMPORT_MAPPING.items()}

NAME_MAPPING = {
    'copy_reg._reconstructor': 'copyreg._reconstructor',
    '__builtin__.xrange': 'builtins.range',
    '__builtin__.unicode': 'builtins.str',
    '__builtin__.unichr': 'builtins.chr',
    '__builtin__.long': 'builtins.int',
    '__builtin__.file': 'builtins.open',
    '__builtin__.basestring': 'builtins.str',
    '__builtin__.reduce': 'functools.reduce',
}

REVERSE_NAME_MAPPING = {v: k for k, v in NAME_MAPPING.items()}
