"""NT path handling stub."""
import os


def abspath(path):
    return os.path.abspath(path)

def normpath(path):
    return path

# Real CPython 3.12+ `ntpath.splitroot` — was missing entirely
# (`AttributeError`), breaking `nturl2path.py`'s `pathname2url` (used by
# `urllib.request` for `file://` URL construction on Windows-style paths) and
# its own `test_nturl2path.py`. Splits into (drive, root, tail): drive is a
# leading drive letter (`C:`) or UNC share (`\\server\share`), root is the
# path separator marking an absolute path (empty for a relative one), tail
# is everything else.
def splitroot(p):
    sep = '\\'
    altsep = '/'
    colon = ':'
    unc_prefix = '\\\\?\\unc\\'
    empty = ''
    normp = p.replace(altsep, sep)
    if normp[:1] == sep:
        if normp[1:2] == sep:
            start = 8 if normp[:8].lower() == unc_prefix else 2
            index = normp.find(sep, start)
            if index == -1:
                return p, empty, empty
            index2 = normp.find(sep, index + 1)
            if index2 == -1:
                return p, empty, empty
            return p[:index2], p[index2:index2+1], p[index2+1:]
        else:
            return empty, p[:1], p[1:]
    elif normp[1:2] == colon:
        if normp[2:3] == sep:
            return p[:2], p[2:3], p[3:]
        else:
            return p[:2], empty, p[2:]
    else:
        return empty, empty, p

def splitdrive(path):
    drive, root, tail = splitroot(path)
    return drive, root + tail

def splitunc(path):
    return '', path

def isabs(path):
    return path.startswith('/') or path.startswith('\\')

def exists(path):
    return os.path.exists(path)

def lexists(path):
    return os.path.exists(path)

def isdir(path):
    return os.path.isdir(path)

def isfile(path):
    return os.path.isfile(path)

def islink(path):
    return os.path.islink(path)

def ismount(path):
    return True

def samefile(f1, f2):
    return os.stat(f1) == os.stat(f2)

def sameopenfile(f1, f2):
    return True

def samestat(s1, s2):
    return s1 == s2
