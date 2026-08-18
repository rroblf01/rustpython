r"""HTTP/1.1 client library.

This is a minimal implementation providing the basic HTTP client classes.
"""

import email.message
import io
import http
import http.client

# HTTP status codes and messages
from http import HTTPStatus

class HTTPException(Exception):
    """Base class for HTTP exceptions."""
    pass

class NotConnected(HTTPException):
    """Raised when trying to use an unconnected HTTP connection."""
    pass

class InvalidURL(HTTPException):
    """Raised when an URL is invalid."""
    def __init__(self, url, msg=None):
        if msg is None:
            msg = f"invalid URL: {url!r}"
        super().__init__(msg)
        self.url = url

class CannotSendRequest(HTTPException):
    """Raised when trying to send a request in the wrong state."""
    pass

class BadStatusLine(HTTPException):
    """Raised when a server response has a bad status line."""
    def __init__(self, line="", url=None, error=None):
        if not line:
            line = ""
        super().__init__(repr(line))
        self.lines = line
        self.url = url
        self.error = error

class HTTPResponse(io.BufferedIOBase):
    """An HTTP response from a server."""

    def __init__(self, sock, debuglevel=0, method=None, url=None):
        self.fp = sock.makefile("rb")
        self.debuglevel = debuglevel
        self._method = method
        self.url = url
        self._headers = []
        self._body = None
        self.status = None
        self.reason = None
        self.length = None
        self._closed = False
        self.msg = None

    def read(self, amt=None):
        if self.fp is None:
            return b""
        if self.length is not None:
            data = self.fp.read(self.length)
            self.length = 0
        else:
            data = self.fp.read()
        return data

    def getheader(self, name, default=None):
        for header, value in self._headers:
            if header.lower() == name.lower():
                return value
        return default

    def getheaders(self):
        return self._headers

    def close(self):
        if not self._closed:
            self._closed = True
            if self.fp:
                self.fp.close()
                self.fp = None

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

class HTTPConnection:
    """An HTTP connection to a server."""

    default_port = 80

    def __init__(self, host, port=None, timeout=30, source_address=None):
        self.host = host
        self.port = port or self.default_port
        self.timeout = timeout
        self.source_address = source_address
        self._tunnel_host = None
        self._tunnel_port = None
        self._conn = None

    def request(self, method, url, body=None, headers=None, encode_chunked=False):
        """Send an HTTP request."""
        if headers is None:
            headers = {}
        self._method = method
        self._path = url
        self._headers = headers
        self._body = body

    def getresponse(self):
        """Get the HTTP response."""
        return HTTPResponse(self._conn, method=self._method, url=self._path)

    def set_debuglevel(self, level):
        self.debuglevel = level

    def close(self):
        if self._conn:
            self._conn.close()
            self._conn = None

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()


class HTTPSConnection(HTTPConnection):
    """An HTTPS connection to a server."""
    default_port = 443

    def __init__(self, host, port=None, key_file=None, cert_file=None,
                 timeout=30, source_address=None, *, context=None,
                 check_hostname=None, blocksize=8192):
        super().__init__(host, port, timeout, source_address)
        self.key_file = key_file
        self.cert_file = cert_file
        self.context = context
        if check_hostname is not None:
            self.check_hostname = check_hostname
        self.blocksize = blocksize


def parse_headers(fp, _class=HTTPMessage):
    """Read HTTP headers from a file pointer."""
    headers = []
    while True:
        line = fp.readline()
        if not line or line == b"\r\n" or line == b"\n":
            break
        line = line.decode("utf-8", errors="replace").rstrip("\r\n")
        if ":" in line:
            name, value = line.split(":", 1)
            headers.append((name.strip(), value.strip()))
    return _class(headers)


class HTTPMessage(email.message.Message):
    """HTTP message class for headers."""

    def getallmatchingheaders(self, name):
        name = name.lower()
        results = []
        for header, value in self._headers:
            if header.lower() == name:
                results.append((header, value))
        return results

    def __init__(self, headers=None):
        super().__init__()
        self._headers = headers or []
        for name, value in self._headers:
            self[name] = value
