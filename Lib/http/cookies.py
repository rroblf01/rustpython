"""HTTP cookie handling.

Minimal implementation of http.cookies.SimpleCookie and Morsel,
sufficient for Django and common WSGI use.
"""

import re
import string

__all__ = ["SimpleCookie", "Morsel", "CookieError"]

# ── Exception ────────────────────────────────────────────────────────────────


class CookieError(Exception):
    """Invalid cookie data."""
    pass


# ── Morsel (a single cookie key-value pair with attributes) ──────────────────

# Legal attribute keys in a Set-Cookie header
_RESERVED = {
    "expires", "path", "comment", "domain", "max-age", "secure",
    "httponly", "version", "samesite",
}


class Morsel(dict):
    """A key/value pair with additional cookie attributes.

    Keys in the dict represent cookie attributes like 'path', 'domain',
    'expires', etc.
    """

    def __init__(self):
        # Note: dict.__init__ not called (RustPython limitation)
        self._key = None
        self._value = None
        self._coded_value = None

    def set(self, key, value, coded_value):
        """Set the key, value, and coded_value of the morsel."""
        self._key = key
        self._value = value
        self._coded_value = coded_value

    def __setitem__(self, key, value):
        """Set an attribute on the cookie."""
        dict.__setitem__(self, key, value)

    def __getitem__(self, key):
        """Get an attribute value."""
        return dict.__getitem__(self, key)

    def __str__(self):
        """Return the plain (uncoded) value."""
        return self._value if self._value is not None else ""

    def key(self):
        return self._key

    def value(self):
        return self._value

    def coded_value(self):
        return self._coded_value

    def OutputString(self, attrs=None):
        """Return a string representation suitable for a Set-Cookie header.

        If attrs is provided, it's a list of attribute names to include.
        """
        result = self._coded_value if self._coded_value is not None else ""

        if attrs is None:
            # Default: output all preserved attributes in canonical order
            keys = []
            for k in ("expires", "path", "comment", "domain", "max-age",
                      "secure", "httponly", "version", "samesite"):
                if k in self:
                    keys.append(k)
            # Also include any custom attributes
            for k in self:
                if k not in keys:
                    keys.append(k)
        else:
            keys = [a.lower() for a in attrs if a.lower() in self]

        for k in keys:
            v = self[k]
            if v is not None and v is not False:
                if k in ("secure", "httponly") and v is True:
                    result += "; " + k
                else:
                    result += "; " + k + "=" + str(v)
            elif k in ("secure", "httponly") and v is True:
                result += "; " + k

        return result

    def __repr__(self):
        return "<Morsel: {}={}>".format(self._key, self._value)


# ── Cookie (a collection of Morsels) ─────────────────────────────────────────


class BaseCookie(dict):
    """A container for cookies represented as Morsel objects."""

    def __init__(self, input=None):
        # Note: dict.__init__ not called (RustPython limitation)
        if input is not None:
            self.load(input)

    def __setitem__(self, key, value):
        """Set a cookie's value.

        If value is a Morsel, it's stored directly.
        Otherwise, a new Morsel is created.
        """
        if isinstance(value, Morsel):
            dict.__setitem__(self, key, value)
        else:
            m = self._make_morsel()
            m.set(key, str(value), str(value))
            dict.__setitem__(self, key, m)

    def __getitem__(self, key):
        return dict.__getitem__(self, key).value()

    def __str__(self):
        """Return the cookies as a Cookie (request) header value."""
        return "; ".join(str(m) for m in self.values())

    def output(self, attrs=None, header="Set-Cookie:", sep="\n"):
        """Return a string suitable for HTTP response headers."""
        result = []
        for key, morsel in self.items():
            result.append(header + " " + morsel.OutputString(attrs))
        return sep.join(result)

    def js_output(self, attrs=None):
        """Return JavaScript code to set the cookies."""
        result = []
        for key, morsel in self.items():
            js_code = (
                'document.cookie = "'
                + morsel.OutputString(attrs).replace('"', '\\"')
                + '";'
            )
            result.append(js_code)
        return "\n".join(result)

    def _make_morsel(self):
        return Morsel()

    def load(self, rawdata):
        """Load cookies from a raw HTTP Cookie header string."""
        if isinstance(rawdata, dict):
            for key, val in rawdata.items():
                self[key] = val
            return

        rawdata = str(rawdata)
        if not rawdata:
            return

        for pair in rawdata.split(";"):
            pair = pair.strip()
            if not pair:
                continue
            if "=" in pair:
                key, val = pair.split("=", 1)
                key = key.strip()
                val = val.strip()
            else:
                key = pair.strip()
                val = ""
            self[key] = val


class SimpleCookie(BaseCookie):
    """A simple RFC 6265 cookie implementation."""
    pass


# ── Backward compatibility aliases ───────────────────────────────────────────

# http.cookies._CookiePattern is used by Django in some places
_CookiePattern = re.compile(r'([^=;]+)=([^;]*)')
