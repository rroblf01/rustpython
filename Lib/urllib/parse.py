"""URL parsing and encoding utilities.

Minimal implementation sufficient for Django and common stdlib use.
"""

import re

__all__ = [
    "urlparse", "urlunparse", "urljoin", "urlsplit", "urlunsplit",
    "quote", "quote_plus", "unquote", "unquote_plus",
    "parse_qs", "parse_qsl", "urlencode",
]


# ── Result types ─────────────────────────────────────────────────────────────


class ParseResult:
    """URL parsed into 6 components: scheme, netloc, path, params, query, fragment."""

    def __init__(self, scheme, netloc, path, params, query, fragment):
        self.scheme = scheme
        self.netloc = netloc
        self.path = path
        self.params = params
        self.query = query
        self.fragment = fragment

    def geturl(self):
        return urlunparse(self)


class SplitResult:
    """URL split into 5 components: scheme, netloc, path, query, fragment."""

    def __init__(self, scheme, netloc, path, query, fragment):
        self.scheme = scheme
        self.netloc = netloc
        self.path = path
        self.query = query
        self.fragment = fragment

    def geturl(self):
        return urlunsplit(self)


# ── URL Splitting / Parsing ──────────────────────────────────────────────────


def urlsplit(url, scheme="", allow_fragments=True):
    """Parse a URL into 5 components: (scheme, netloc, path, query, fragment)."""
    url = str(url)
    fragment = ""

    if allow_fragments and "#" in url:
        parts = url.rpartition("#")
        url, fragment = parts[0], parts[2]

    query = ""
    scheme_part = scheme

    # Extract scheme
    i = url.find(":")
    if i > 0:
        valid_scheme = True
        for c in url[:i]:
            if c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-.":
                valid_scheme = False
                break
        if valid_scheme:
            scheme_part = url[:i]
            url = url[i + 1:]
            # Eat //
            if url.startswith("//"):
                url = url[2:]

    # Extract netloc (host:port) — everything before first /, ?, or #
    netloc = ""
    path = url
    path_start = len(url)
    for delim in ("/", "?", "#"):
        d = url.find(delim)
        if d >= 0 and d < path_start:
            path_start = d
    if path_start > 0 and path_start < len(url):
        netloc = url[:path_start]
        path = url[path_start:]
    elif path_start > 0 and path_start == len(url):
        netloc = url
        path = ""

    if "?" in path:
        path, query = path.split("?", 1)

    return SplitResult(scheme_part, netloc, path, query, fragment)


def urlunsplit(parts):
    """Combine the elements of a SplitResult into a URL string."""
    scheme = parts.scheme
    netloc = parts.netloc
    path = parts.path
    query = parts.query
    fragment = parts.fragment
    result = ""
    if scheme:
        result += scheme + ":"
    if netloc:
        result += "//" + netloc
    result += path
    if query:
        result += "?" + query
    if fragment:
        result += "#" + fragment
    return result


def urlparse(url, scheme="", allow_fragments=True):
    """Parse a URL into 6 components: (scheme, netloc, path, params, query, fragment)."""
    split = urlsplit(url, scheme, allow_fragments)
    path = split.path
    params = ""

    # Extract params (semicolon-separated after path)
    if ";" in path:
        path, params = path.split(";", 1)

    return ParseResult(split.scheme, split.netloc, path, params, split.query, split.fragment)


def urlunparse(parts):
    """Combine the elements of a ParseResult into a URL string."""
    scheme = parts.scheme
    netloc = parts.netloc
    path = parts.path
    params = parts.params
    query = parts.query
    fragment = parts.fragment
    if params:
        path = path + ";" + params
    return urlunsplit(SplitResult(scheme, netloc, path, query, fragment))


def urljoin(base, url, allow_fragments=True):
    """Construct a full (absolute) URL by combining a base URL with another URL."""
    url = str(url)
    if not url:
        return base

    # If url has a scheme, it's absolute
    if "://" in url or url.startswith("//"):
        p = urlsplit(url, allow_fragments=allow_fragments)
        if p.scheme or url.startswith("//"):
            return url

    base_parts = urlsplit(base, allow_fragments=allow_fragments)

    # If url starts with /, replace path from base
    if url.startswith("/"):
        return urlunsplit(SplitResult(base_parts.scheme, base_parts.netloc, url, "", ""))

    # If url is a query string, append to base path
    if url.startswith("?"):
        return urlunsplit(SplitResult(base_parts.scheme, base_parts.netloc,
                                      base_parts.path, url[1:], ""))

    # If url has a fragment only
    if url.startswith("#"):
        return urlunsplit(SplitResult(base_parts.scheme, base_parts.netloc,
                                      base_parts.path, base_parts.query, url[1:]))

    # Relative URL: resolve against base's directory
    base_path = base_parts.path or "/"
    if not base_path.endswith("/"):
        if "/" in base_path:
            base_path = base_path.rpartition("/")[0] + "/"
        else:
            base_path = "/"

    # Resolve .. and . in the combined path
    combined = base_path + url
    segments = combined.split("/")
    result = []
    for seg in segments:
        if seg == "..":
            if result:
                result.pop()
        elif seg == "." or seg == "":
            continue
        else:
            result.append(seg)
    # Reconstruct path
    if combined.startswith("/"):
        resolved_path = "/" + "/".join(result)
    else:
        resolved_path = "/".join(result)

    # Ensure leading /
    if base_path.startswith("/") and not resolved_path.startswith("/"):
        resolved_path = "/" + resolved_path

    return urlunsplit(SplitResult(base_parts.scheme, base_parts.netloc, resolved_path, "", ""))


# ── Quote / Unquote ──────────────────────────────────────────────────────────

_ALWAYS_SAFE = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.-~"


def quote(string, safe="/", encoding=None, errors=None):
    """Percent-encode a string, replacing special characters with %XX escapes."""
    string = str(string)
    safe_chars = _ALWAYS_SAFE + safe

    result = []
    for c in string:
        if c in safe_chars:
            result.append(c)
        elif c == " ":
            result.append("%20")
        else:
            code = ord(c)
            if code < 128:
                result.append("%%%02X" % code)
            else:
                for byte in c.encode("utf-8"):
                    result.append("%%%02X" % byte)
    return "".join(result)


def quote_plus(string, safe="", encoding=None, errors=None):
    """Like quote(), but also replaces spaces with '+'."""
    return quote(string, safe + " ", encoding, errors).replace(" ", "+")


def unquote(string, encoding="utf-8", errors="replace"):
    """Replace %XX escapes with their single-character equivalent."""
    string = str(string)
    if "%" not in string:
        return string

    result = []
    i = 0
    n = len(string)
    while i < n:
        c = string[i]
        if c == "%" and i + 2 < n:
            h = string[i + 1:i + 3]
            try:
                code = int(h, 16)
                result.append(chr(code))
                i += 3
                continue
            except ValueError:
                pass
        result.append(c)
        i += 1
    return "".join(result)


def unquote_plus(string, encoding="utf-8", errors="replace"):
    """Like unquote(), but also replaces '+' with spaces."""
    if "+" in string:
        string = string.replace("+", " ")
    return unquote(string, encoding, errors)


# ── Query string parsing ─────────────────────────────────────────────────────


def parse_qs(qs, keep_blank_values=False, strict_parsing=False, encoding="utf-8",
             errors="replace", max_num_fields=None, separator="&"):
    """Parse a query string into a dict of lists."""
    result = {}
    if not qs:
        return result

    qs = str(qs)
    for param in qs.split(separator):
        if not param and strict_parsing:
            raise ValueError("empty parameter in query string")

        if "=" in param:
            key, val = param.split("=", 1)
        else:
            key, val = param, ""

        key = unquote_plus(key, encoding, errors)
        val = unquote_plus(val, encoding, errors)

        if keep_blank_values or val:
            result.setdefault(key, []).append(val)
    return result


def parse_qsl(qs, keep_blank_values=False, strict_parsing=False, encoding="utf-8",
              errors="replace", max_num_fields=None, separator="&"):
    """Parse a query string into a list of (key, value) pairs."""
    result = []
    if not qs:
        return result

    qs = str(qs)
    for param in qs.split(separator):
        if not param and strict_parsing:
            raise ValueError("empty parameter in query string")

        if "=" in param:
            key, val = param.split("=", 1)
        else:
            key, val = param, ""

        key = unquote_plus(key, encoding, errors)
        val = unquote_plus(val, encoding, errors)

        if keep_blank_values or val:
            result.append((key, val))
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


# ── Helper: URL-to-query-string delimiter ────────────────────────────────────


def urldefrag(url):
    """Remove fragment from URL, returning (url, fragment)."""
    url = str(url)
    if "#" in url:
        parts = url.rpartition("#")
        return parts[0], parts[2]
    return url, ""
