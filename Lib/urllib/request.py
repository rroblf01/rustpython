"""Minimal urllib.request.

Real CPython's urllib.request is a large framework of openers, handlers,
and protocol adapters. This is a pragmatic subset covering the common
case — `urlopen(url_or_request)` over plain HTTP/HTTPS via `http.client`
— since this interpreter's `http.client` itself only exposes a bare
status/read() response (no header parsing) to build a full opener stack
on top of.
"""

import http.client
from urllib.parse import urlsplit
from urllib.error import URLError, HTTPError

__all__ = ["Request", "urlopen", "urlretrieve", "pathname2url", "url2pathname"]


class Request:
    def __init__(self, url, data=None, headers=None, method=None):
        self.full_url = url
        self.data = data
        self.headers = dict(headers) if headers else {}
        self.method = method

    def add_header(self, key, val):
        self.headers[key] = val

    def get_method(self):
        if self.method is not None:
            return self.method
        return "POST" if self.data is not None else "GET"

    def get_full_url(self):
        return self.full_url


class _Response:
    def __init__(self, raw, url, status):
        self._raw = raw
        self._url = url
        self.status = status
        self.code = status

    def read(self, *args, **kwargs):
        return self._raw.read(*args, **kwargs)

    def close(self):
        pass

    def getcode(self):
        return self.status

    def geturl(self):
        return self._url

    def info(self):
        return {}

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def urlopen(url, data=None, timeout=None):
    if isinstance(url, Request):
        req = url
    else:
        req = Request(url, data=data)

    parts = urlsplit(req.get_full_url())
    if parts.scheme not in ("http", "https"):
        raise URLError("unsupported protocol: %r" % parts.scheme)

    host = parts.hostname
    port = parts.port or (443 if parts.scheme == "https" else 80)
    conn = http.client.HTTPConnection(host, port)
    path = parts.path or "/"
    if parts.query:
        path += "?" + parts.query

    body = req.data
    conn.request(req.get_method(), path, body=body)
    resp = conn.getresponse()

    if resp.status >= 400:
        raise HTTPError(req.get_full_url(), resp.status, "HTTP Error", {}, resp)

    return _Response(resp, req.get_full_url(), resp.status)


def urlretrieve(url, filename=None, reporthook=None, data=None):
    with urlopen(url, data=data) as resp:
        content = resp.read()
    if filename is None:
        raise ValueError("filename is required")
    with open(filename, "wb") as f:
        f.write(content)
    return filename, {}


def pathname2url(path):
    return path.replace("\\", "/")


def url2pathname(path):
    return path
