"""OS X support module stub."""


def _get_system_version():
    """Get macOS version."""
    return '10.16'


def _get_platform_osx():
    """Get platform-specific configuration."""
    return []


def _get_cxx_stdlib_for_arch(arch):
    """Get C++ stdlib for architecture."""
    return 'c++'
