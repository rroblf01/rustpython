"""faulthandler module stub."""
import os
import signal


def dump_traceback(file=None, all_threads=True):
    """Dump traceback of all threads."""
    if file is None:
        file = os.sys.stderr


def enable(file=None, all_threads=True):
    """Enable fault handler."""
    pass


def disable():
    """Disable fault handler."""
    pass


def is_enabled():
    return False


def register(signum, file=None, all_threads=True, chain=False):
    """Register a handler for a signal."""
    pass


def cancel_dump_traceback_later():
    pass


def _dump_backtrace(signum, frame):
    pass


def _sigsegv():
    raise RuntimeError('_sigsegv not available')


def _sigfpe():
    raise RuntimeError('_sigfpe not available')


def _fatal_error_c_thread():
    raise RuntimeError('_fatal_error_c_thread not available')


# Alias for backward compatibility
dump_traceback_later = dump_traceback
cancel_dump_traceback_later = cancel_dump_traceback_later
