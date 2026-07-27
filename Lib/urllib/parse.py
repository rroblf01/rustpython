"""URL parsing and encoding utilities.

Minimal implementation sufficient for Django and common stdlib use.
"""

__all__ = [
    "urlparse", "urlunparse", "urljoin", "urlsplit", "urlunsplit",
    "urldefrag", "ParseResult", "SplitResult", "DefragResult",
    "quote", "quote_plus", "quote_from_bytes",
    "unquote", "unquote_plus", "unquote_to_bytes",
    "parse_qs", "parse_qsl", "urlencode",
    "splittype", "splithost", "splituser", "splitpasswd", "splitport",
    "splitnport", "splitquery", "splittag", "splitattr", "splitvalue",
    "unwrap",
]

# Schemes for which an empty authority (netloc) still gets a `//` prefix on
# reconstruction, and schemes urljoin() will actually resolve relatively —
# same lists real CPython's urllib.parse uses.
uses_netloc = ["", "ftp", "http", "gopher", "nntp", "telnet", "imap", "wais",
               "file", "mms", "https", "shttp", "snews", "prospero", "rtsp",
               "rtspu", "rsync", "svn", "svn+ssh", "sftp", "nfs", "git",
               "git+ssh", "ws", "wss"]

uses_relative = ["", "ftp", "http", "gopher", "nntp", "imap", "wais", "file",
                  "https", "shttp", "mms", "prospero", "rtsp", "rtspu",
                  "sftp", "svn", "svn+ssh", "ws", "wss"]

_SCHEME_CHARS = ("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
                 "0123456789+-.")


# ── Result types ─────────────────────────────────────────────────────────────
#
# Plain classes with hand-written __getitem__/__iter__/__len__/__eq__ rather
# than collections.namedtuple subclasses — subclassing a namedtuple doesn't
# properly inherit tuple-like indexing/iteration in this interpreter yet
# (confirmed via a minimal repro: `class Foo(namedtuple(...)): pass` then
# `Foo(1, 2)[0]` raises `KeyError: 0` instead of returning the field). Real
# CPython code and tests interchangeably use both attribute access
# (`result.scheme`) AND tuple unpacking/indexing (`scheme, netloc, ... =
# urlsplit(url)`, `result[0]`), so both must work.


class _URLResultMixin:
    _fields = ()

    def __getitem__(self, i):
        return tuple(self)[i]

    def __iter__(self):
        return iter(getattr(self, f) for f in self._fields)

    def __len__(self):
        return len(self._fields)

    def __eq__(self, other):
        try:
            return tuple(self) == tuple(other)
        except TypeError:
            return NotImplemented

    def __ne__(self, other):
        result = self.__eq__(other)
        if result is NotImplemented:
            return result
        return not result

    def __hash__(self):
        return hash(tuple(self))

    def __repr__(self):
        body = ", ".join("%s=%r" % (f, getattr(self, f)) for f in self._fields)
        return "%s(%s)" % (type(self).__name__, body)


class _NetlocResultMixin(_URLResultMixin):
    @property
    def username(self):
        userinfo = self._userinfo()[0]
        return userinfo

    @property
    def password(self):
        return self._userinfo()[1]

    @property
    def hostname(self):
        host = self._hostinfo()[0]
        return host.lower() if host else host

    @property
    def port(self):
        port = self._hostinfo()[1]
        if port is None:
            return None
        try:
            port_int = int(port, 10)
        except ValueError:
            raise ValueError("Port could not be cast to integer value") from None
        if not (0 <= port_int <= 65535):
            raise ValueError("Port out of range 0-65535")
        return port_int

    def _userinfo(self):
        netloc = self.netloc
        userinfo, have_info, hostinfo = netloc.rpartition("@")
        if have_info:
            username, have_password, password = userinfo.partition(":")
            if not have_password:
                password = None
        else:
            username = password = None
        return username, password

    def _hostinfo(self):
        netloc = self.netloc
        _, _, hostinfo = netloc.rpartition("@")
        if "[" in hostinfo and "]" in hostinfo:
            hostname, _, port = hostinfo.rpartition("]")
            hostname += "]"
            _, _, port = port.partition(":")
        else:
            hostname, have_port, port = hostinfo.rpartition(":")
            if not have_port:
                hostname = port
                port = None
        if not port:
            port = None
        return hostname, port


class ParseResult(_NetlocResultMixin):
    """URL parsed into 6 components: scheme, netloc, path, params, query, fragment."""

    _fields = ("scheme", "netloc", "path", "params", "query", "fragment")

    def __init__(self, scheme, netloc, path, params, query, fragment):
        self.scheme = scheme
        self.netloc = netloc
        self.path = path
        self.params = params
        self.query = query
        self.fragment = fragment

    def geturl(self):
        return urlunparse(self)


class SplitResult(_NetlocResultMixin):
    """URL split into 5 components: scheme, netloc, path, query, fragment."""

    _fields = ("scheme", "netloc", "path", "query", "fragment")

    def __init__(self, scheme, netloc, path, query, fragment):
        self.scheme = scheme
        self.netloc = netloc
        self.path = path
        self.query = query
        self.fragment = fragment

    def geturl(self):
        return urlunsplit(self)


class DefragResult(_URLResultMixin):
    """URL split into (url, fragment)."""

    _fields = ("url", "fragment")

    def __init__(self, url, fragment):
        self.url = url
        self.fragment = fragment

    def geturl(self):
        if self.fragment:
            return self.url + "#" + self.fragment
        return self.url


# ── URL Splitting / Parsing ──────────────────────────────────────────────────


def _splitparams(path):
    if "/" in path:
        i = path.find(";", path.rfind("/"))
        if i < 0:
            return path, ""
    else:
        i = path.find(";")
        if i < 0:
            return path, ""
    return path[:i], path[i + 1:]


def urlsplit(url, scheme="", allow_fragments=True):
    """Parse a URL into 5 components: (scheme, netloc, path, query, fragment)."""
    url = str(url)
    scheme = str(scheme)
    fragment = ""

    if allow_fragments and "#" in url:
        url, _, fragment = url.partition("#")

    query = ""
    if "?" in url:
        url, _, query = url.partition("?")

    i = url.find(":")
    if i > 0:
        candidate = url[:i]
        if candidate and candidate[0].isalpha() and all(c in _SCHEME_CHARS for c in candidate):
            scheme, url = candidate.lower(), url[i + 1:]

    netloc = ""
    if url[:2] == "//":
        delim = len(url)
        for c in "/?#":
            wdelim = url.find(c, 2)
            if wdelim >= 0:
                delim = min(delim, wdelim)
        netloc, url = url[2:delim], url[delim:]

    return SplitResult(scheme, netloc, url, query, fragment)


def urlunsplit(components):
    """Combine the elements of a 5-item sequence into a URL string."""
    scheme, netloc, url, query, fragment = components
    if netloc or (scheme and scheme in uses_netloc and url[:2] != "//"):
        if url and url[:1] != "/":
            url = "/" + url
        url = "//" + (netloc or "") + url
    if scheme:
        url = scheme + ":" + url
    if query:
        url = url + "?" + query
    if fragment:
        url = url + "#" + fragment
    return url


def urlparse(url, scheme="", allow_fragments=True):
    """Parse a URL into 6 components: (scheme, netloc, path, params, query, fragment)."""
    split = urlsplit(url, scheme, allow_fragments)
    path, params = _splitparams(split.path)
    return ParseResult(split.scheme, split.netloc, path, params, split.query, split.fragment)


def urlunparse(components):
    """Combine the elements of a 6-item sequence into a URL string."""
    scheme, netloc, path, params, query, fragment = components
    if params:
        path = path + ";" + params
    return urlunsplit((scheme, netloc, path, query, fragment))


def urljoin(base, url, allow_fragments=True):
    """Construct a full (absolute) URL by combining a base URL with another URL."""
    base = str(base)
    url = str(url)
    if not base:
        return url
    if not url:
        return base

    bscheme, bnetloc, bpath, bparams, bquery, _ = urlparse(base, "", allow_fragments)
    scheme, netloc, path, params, query, fragment = urlparse(url, bscheme, allow_fragments)

    if scheme != bscheme or scheme not in uses_relative:
        return url
    if scheme in uses_netloc:
        if netloc:
            return urlunparse((scheme, netloc, path, params, query, fragment))
        netloc = bnetloc

    if not path and not params:
        path = bpath
        params = bparams
        if not query:
            query = bquery
        return urlunparse((scheme, netloc, path, params, query, fragment))

    base_parts = bpath.split("/")
    if base_parts[-1] != "":
        del base_parts[-1]

    if path[:1] == "/":
        segments = path.split("/")
    else:
        segments = base_parts + path.split("/")
        segments[1:-1] = filter(None, segments[1:-1])

    resolved_path = []
    for seg in segments:
        if seg == "..":
            if resolved_path:
                resolved_path.pop()
        elif seg == ".":
            continue
        else:
            resolved_path.append(seg)

    if segments[-1] in (".", ".."):
        resolved_path.append("")

    return urlunparse((scheme, netloc, "/".join(resolved_path) or "/", params, query, fragment))


def urldefrag(url):
    """Remove fragment from URL, returning a DefragResult(url, fragment)."""
    url = str(url)
    if "#" in url:
        url, _, frag = url.partition("#")
        return DefragResult(url, frag)
    return DefragResult(url, "")


# ── Quote / Unquote ──────────────────────────────────────────────────────────

_ALWAYS_SAFE = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.-~"


def _to_bytes(string):
    if isinstance(string, (bytes, bytearray)):
        return bytes(string)
    return str(string).encode("utf-8", "surrogatepass")


def quote_from_bytes(bs, safe="/"):
    """Percent-encode raw bytes into a str."""
    if isinstance(bs, str):
        raise TypeError("quote_from_bytes() expected bytes")
    if isinstance(safe, (bytes, bytearray)):
        safe = safe.decode("ascii", "ignore")
    safe_chars = _ALWAYS_SAFE + safe
    result = []
    for byte in bs:
        c = chr(byte)
        if c in safe_chars:
            result.append(c)
        else:
            result.append("%%%02X" % byte)
    return "".join(result)


def quote(string, safe="/", encoding=None, errors=None):
    """Percent-encode a string, replacing special characters with %XX escapes."""
    b = _to_bytes(string)
    return quote_from_bytes(b, safe)


def quote_plus(string, safe="", encoding=None, errors=None):
    """Like quote(), but also replaces spaces with '+'."""
    if " " in (string if isinstance(string, str) else string.decode("latin-1")):
        return quote(string, safe + " ", encoding, errors).replace(" ", "+")
    return quote(string, safe, encoding, errors)


def unquote_to_bytes(string):
    """Percent-decode a str or bytes into raw bytes, without touching '+'."""
    if isinstance(string, str):
        string = string.encode("utf-8")
    else:
        string = bytes(string)
    if b"%" not in string:
        return string

    result = bytearray()
    i = 0
    n = len(string)
    while i < n:
        byte = string[i]
        if byte == 0x25 and i + 2 < n:  # '%'
            hex_part = string[i + 1:i + 3].decode("ascii", "replace")
            try:
                code = int(hex_part, 16)
                result.append(code)
                i += 3
                continue
            except ValueError:
                pass
        result.append(byte)
        i += 1
    return bytes(result)


def unquote(string, encoding="utf-8", errors="replace"):
    """Replace %XX escapes with their single-character equivalent."""
    if isinstance(string, bytes):
        return unquote_to_bytes(string).decode(encoding, errors)
    string = str(string)
    if "%" not in string:
        return string
    raw = unquote_to_bytes(string)
    return raw.decode(encoding, errors)


def unquote_plus(string, encoding="utf-8", errors="replace"):
    """Like unquote(), but also replaces '+' with spaces."""
    string = string.replace("+", " ")
    return unquote(string, encoding, errors)


# ── Query string parsing ─────────────────────────────────────────────────────


def parse_qsl(qs, keep_blank_values=False, strict_parsing=False, encoding="utf-8",
              errors="replace", max_num_fields=None, separator="&"):
    """Parse a query string into a list of (key, value) pairs."""
    result = []
    if not qs:
        return result

    as_bytes = isinstance(qs, bytes)
    qs = qs.decode("ascii", "replace") if as_bytes else str(qs)
    sep = separator.decode("ascii") if isinstance(separator, bytes) else separator

    pairs = qs.split(sep)
    if max_num_fields is not None and len(pairs) > max_num_fields:
        raise ValueError("Max number of fields exceeded")

    for name_value in pairs:
        if not name_value and not strict_parsing:
            continue
        nv = name_value.split("=", 1)
        if len(nv) != 2:
            if strict_parsing:
                raise ValueError("bad query field: %r" % (name_value,))
            if keep_blank_values:
                nv.append("")
            else:
                continue
        if len(nv[1]) or keep_blank_values:
            name = unquote_plus(nv[0], encoding=encoding, errors=errors)
            value = unquote_plus(nv[1], encoding=encoding, errors=errors)
            if as_bytes:
                name = name.encode(encoding, errors)
                value = value.encode(encoding, errors)
            result.append((name, value))
    return result


def parse_qs(qs, keep_blank_values=False, strict_parsing=False, encoding="utf-8",
             errors="replace", max_num_fields=None, separator="&"):
    """Parse a query string into a dict of lists."""
    result = {}
    for name, value in parse_qsl(qs, keep_blank_values, strict_parsing, encoding,
                                  errors, max_num_fields, separator):
        result.setdefault(name, []).append(value)
    return result


# ── URL Encoding ─────────────────────────────────────────────────────────────


def urlencode(query, doseq=False, safe="", encoding=None, errors=None, quote_via=quote):
    """Encode a mapping or sequence of 2-tuples into a query string."""
    if hasattr(query, "items"):
        query = list(query.items())

    parts = []
    for k, v in query:
        if doseq and isinstance(v, (list, tuple)):
            for item in v:
                parts.append(quote_via(str(k), safe) + "=" + quote_via(str(item), safe))
        else:
            parts.append(quote_via(str(k), safe) + "=" + quote_via(str(v), safe))
    return "&".join(parts)


# ── Legacy (deprecated since Python 3.8) split* helpers ──────────────────────
#
# Simple, undocumented string-splitting functions predating urlsplit; still
# imported by some real-world code (and by CPython's own test_urlparse.py).


def splittype(url):
    i = url.find(":")
    if i > 0 and "/" not in url[:i]:
        return url[:i].lower(), url[i + 1:]
    return None, url


def splithost(url):
    if url[:2] == "//":
        rest = url[2:]
        i = len(rest)
        for c in "/#?":
            j = rest.find(c)
            if j >= 0:
                i = min(i, j)
        return rest[:i], rest[i:]
    return None, url


def splituser(host):
    user, delim, host = host.rpartition("@")
    return (user if delim else None), host


def splitpasswd(user):
    user, delim, passwd = user.partition(":")
    return user, (passwd if delim else None)


def splitport(host):
    i = host.rfind(":")
    if i >= 0:
        port = host[i + 1:]
        if port.isdigit() or port == "":
            if port:
                return host[:i], port
            return host[:i], None
    return host, None


def splitnport(host, defport=-1):
    host, port = splitport(host)
    if not port:
        return host, defport
    if port.isdigit() and port.isascii():
        return host, int(port)
    return host, None


def splitquery(url):
    path, delim, query = url.rpartition("?")
    if delim:
        return path, query
    return url, None


def splittag(url):
    path, delim, tag = url.rpartition("#")
    if delim:
        return path, tag
    return url, None


def splitattr(url):
    words = url.split(";")
    return words[0], words[1:]


def splitvalue(attr):
    attr, delim, value = attr.partition("=")
    return attr, (value if delim else None)


def unwrap(url):
    url = str(url).strip()
    if url[:1] == "<" and url[-1:] == ">":
        url = url[1:-1].strip()
    if url[:4] == "URL:":
        url = url[4:].strip()
    return url
