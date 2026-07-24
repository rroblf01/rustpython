"""File input module."""

import sys


def input(files=None, inplace=False, backup='', *, mode='r', openhook=None):
    """Return a FileInput instance."""
    return FileInput(files, inplace, backup, mode=mode, openhook=openhook)


class FileInput:
    def __init__(self, files=None, inplace=False, backup='', *, mode='r', openhook=None):
        if files is None:
            files = ('-',)
        if isinstance(files, str):
            files = (files,)
        self._files = files
        self._inplace = inplace
        self._backup = backup
        self._mode = mode
        self._openhook = openhook
        self._file = None
        self._index = -1
        self._lineno = 0
        self._filename = ''

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def __iter__(self):
        return self

    def __next__(self):
        line = self.readline()
        if not line:
            raise StopIteration
        return line

    def __getitem__(self, i):
        if i != self._lineno:
            raise TypeError('FileInput indices must be sequential')
        return self.next()

    def next(self):
        return self.__next__()

    def readline(self):
        while True:
            if self._file is None:
                self._index += 1
                if self._index >= len(self._files):
                    return ''
                self._filename = self._files[self._index]
                if self._filename == '-':
                    self._file = sys.stdin
                else:
                    self._file = open(self._filename, self._mode)
            line = self._file.readline()
            if line:
                self._lineno += 1
                return line
            self._file.close()
            self._file = None
            return ''

    def filename(self):
        return self._filename

    def lineno(self):
        return self._lineno

    def filelineno(self):
        if self._file:
            return self._file.tell()
        return 0

    def fileno(self):
        if self._file:
            return self._file.fileno()
        return -1

    def isfirstline(self):
        return self._lineno == 1

    def close(self):
        if self._file:
            self._file.close()
            self._file = None
