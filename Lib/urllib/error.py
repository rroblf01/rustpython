"""Exception classes raised by urllib.

The base classes mirror real CPython's urllib.error: URLError wraps a
low-level reason (a string or another exception), and HTTPError is both
an URLError *and* a valid (minimal) file-like response object, since
real code sometimes catches HTTPError and reads it like a response.
"""

from io import UnsupportedOperation


__all__ = ["URLError", "HTTPError", "ContentTooShortError"]


class URLError(OSError):
    def __init__(self, reason, filename=None):
        self.reason = reason
        self.filename = filename
        if filename is not None:
            super().__init__(reason, filename)
        else:
            super().__init__(reason)

    def __str__(self):
        return "<urlopen error %s>" % (self.reason,)


class HTTPError(URLError):
    """Raised when HTTP error occurs, but also acts like a file object."""

    def __init__(self, url, code, msg, hdrs, fp):
        self.code = code
        self.msg = msg
        self.hdrs = hdrs
        self.fp = fp
        self.filename = url
        URLError.__init__(self, msg)

    @property
    def reason(self):
        return self.msg

    @property
    def headers(self):
        return self.hdrs

    @headers.setter
    def headers(self, headers):
        self.hdrs = headers

    def __str__(self):
        return "HTTP Error %s: %s" % (self.code, self.msg)

    def __repr__(self):
        return "<HTTPError %s: %r>" % (self.code, self.msg)

    def read(self, *args, **kwargs):
        if self.fp is None:
            raise UnsupportedOperation("no response body")
        return self.fp.read(*args, **kwargs)

    def close(self):
        if self.fp is not None:
            self.fp.close()

    def geturl(self):
        return self.filename

    def getcode(self):
        return self.code

    def info(self):
        return self.hdrs


class ContentTooShortError(URLError):
    def __init__(self, message, content):
        URLError.__init__(self, message)
        self.content = content
