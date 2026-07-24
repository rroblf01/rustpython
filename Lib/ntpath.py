"""NT path handling stub."""
import os


def abspath(path):
    return os.path.abspath(path)

def normpath(path):
    return path

def splitdrive(path):
    return '', path

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
