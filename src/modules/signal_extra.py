from enum import IntEnum


class Signals(IntEnum):
    """An enumeration of signal numbers and their names.

    This mirrors CPython's signal.Signals enum.
    """
    SIGABRT = 6
    SIGALRM = 14
    SIGBUS = 7
    SIGCHLD = 17
    SIGCONT = 18
    SIGFPE = 8
    SIGHUP = 1
    SIGILL = 4
    SIGINT = 2
    SIGIO = 29
    SIGIOT = 6
    SIGKILL = 9
    SIGPIPE = 13
    SIGPROF = 27
    SIGQUIT = 3
    SIGSEGV = 11
    SIGSTOP = 19
    SIGSYS = 31
    SIGTERM = 15
    SIGTRAP = 5
    SIGTSTP = 20
    SIGTTIN = 21
    SIGTTOU = 22
    SIGURG = 23
    SIGUSR1 = 10
    SIGUSR2 = 12
    SIGVTALRM = 26
    SIGXCPU = 24
    SIGXFSZ = 25
    SIGWINCH = 28
    SIGBREAK = 21
    SIGABRT = 6


class Handlers(IntEnum):
    """An enumeration of signal handler values."""
    SIG_DFL = 0
    SIG_IGN = 1
