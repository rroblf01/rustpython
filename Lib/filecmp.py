"""Compare files efficiently."""

import os
import stat
from itertools import filterfalse


def _sig(st):
    return (stat.S_IFMT(st.st_mode), st.st_size, st.st_mtime)


def cmp(f1, f2, shallow=True):
    """Compare two files."""
    s1 = os.stat(f1)
    s2 = os.stat(f2)
    if s1.st_ino == s2.st_ino and s1.st_dev == s2.st_dev:
        return True
    if shallow:
        return _sig(s1) == _sig(s2)
    if _sig(s1) != _sig(s2):
        return False
    with open(f1, 'rb') as fp1, open(f2, 'rb') as fp2:
        b1 = fp1.read()
        b2 = fp2.read()
    return b1 == b2


class dircmp:
    """Compare directories."""

    def __init__(self, a, b, ignore=None, hide=None):
        self.left = a
        self.right = b
        if hide is None:
            hide = [os.curdir, os.pardir]
        self.hide = hide
        if ignore is None:
            ignore = ['RCS', 'CVS', 'tags']
        self.ignore = ignore

    def phase0(self):
        self.left_list = self._filter_files(os.listdir(self.left))
        self.right_list = self._filter_files(os.listdir(self.right))

    def _filter_files(self, names):
        return [x for x in names if x not in self.hide and x not in self.ignore]

    def phase1(self):
        self.common = list(set(self.left_list) & set(self.right_list))
        self.left_only = [x for x in self.left_list if x not in self.common]
        self.right_only = [x for x in self.right_list if x not in self.common]

    def phase2(self):
        self.common_dirs = []
        self.common_files = []
        self.common_funny = []
        for x in self.common:
            a_path = os.path.join(self.left, x)
            b_path = os.path.join(self.right, x)
            ok = True
            try:
                a_stat = os.stat(a_path)
            except OSError:
                a_stat = None
                ok = False
            try:
                b_stat = os.stat(b_path)
            except OSError:
                b_stat = None
                ok = False
            if ok:
                a_type = stat.S_IFMT(a_stat.st_mode)
                b_type = stat.S_IFMT(b_stat.st_mode)
                if a_type != b_type:
                    self.common_funny.append(x)
                elif stat.S_ISDIR(a_type):
                    self.common_dirs.append(x)
                else:
                    self.common_files.append(x)
            else:
                self.common_funny.append(x)

    def phase3(self):
        self.same_files = []
        self.diff_files = []
        self.funny_files = []
        for x in self.common_files:
            a_path = os.path.join(self.left, x)
            b_path = os.path.join(self.right, x)
            try:
                ok = cmp(a_path, b_path, shallow=False)
            except OSError:
                self.funny_files.append(x)
            else:
                if ok:
                    self.same_files.append(x)
                else:
                    self.diff_files.append(x)

    def phase4(self):
        self.subdirs = {}
        for x in self.common_dirs:
            a_path = os.path.join(self.left, x)
            b_path = os.path.join(self.right, x)
            self.subdirs[x] = dircmp(a_path, b_path, self.ignore, self.hide)

    def report(self):
        self.phase0()
        self.phase1()
        self.phase2()
        self.phase3()
        self.phase4()
        self._report()

    def _report(self):
        print(f"diff {self.left} {self.right}")
        if self.left_only:
            print(f"Only in {self.left}: {self.left_only}")
        if self.right_only:
            print(f"Only in {self.right}: {self.right_only}")
        if self.same_files:
            print(f"Identical files: {self.same_files}")
        if self.diff_files:
            print(f"Differing files: {self.diff_files}")
        if self.funny_files:
            print(f"Trouble with common files: {self.funny_files}")
        for sd in self.subdirs.values():
            sd._report()
