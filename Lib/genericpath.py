"""Generic path functions for compatibility."""

import os
import stat


def commonprefix(paths):
    if not paths:
        return ''
    s1 = min(paths)
    s2 = max(paths)
    for i, c in enumerate(s1):
        if c != s2[i]:
            return s1[:i]
    return s1


def commonpath(paths):
    if not paths:
        raise ValueError('commonpath() arg is an empty sequence')
    paths = [os.path.abspath(p) for p in paths]
    prefix = commonprefix(paths)
    if not os.path.isdir(prefix):
        prefix = os.path.dirname(prefix)
    return prefix
