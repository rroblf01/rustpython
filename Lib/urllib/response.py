"""urllib.response module."""


class addinfourl:
    def __init__(self, fp, headers, url, code=None):
        self.fp = fp
        self.headers = headers
        self.url = url
        self.code = code

    def read(self, *args):
        return self.fp.read(*args)

    def readline(self, *args):
        return self.fp.readline(*args)

    def readlines(self, *args):
        return self.fp.readlines(*args)

    def close(self):
        return self.fp.close()

    def info(self):
        return self.headers

    def geturl(self):
        return self.url
