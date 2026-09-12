"""Minimal urllib.request.

Real CPython's urllib.request is a large framework of openers, handlers,
and protocol adapters. This is a pragmatic subset covering the common
case — `urlopen(url_or_request)` over plain HTTP/HTTPS via `http.client`
— since this interpreter's `http.client` itself only exposes a bare
status/read() response (no header parsing) to build a full opener stack
on top of.
"""

import re
import sys
import time
import os
import bisect
import hashlib
import http.client
from urllib.parse import urlsplit, urlparse
from urllib.error import URLError, HTTPError

__all__ = ["Request", "urlopen", "urlretrieve", "pathname2url", "url2pathname",
           "OpenerDirector", "BaseHandler", "HTTPPasswordMgr",
           "HTTPPasswordMgrWithDefaultRealm", "HTTPPasswordMgrWithPriorAuth",
           "AbstractBasicAuthHandler", "HTTPBasicAuthHandler",
           "AbstractDigestAuthHandler"]


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


def urlcleanup():
    """Clear the global URL cache (real CPython's urllib.request.urlcleanup).

    This interpreter keeps no URL cache, so the cleanup is a genuine no-op —
    but test files (e.g. test_robotparser.py) call it in setUpClass/cleanup
    and require it to exist.
    """
    pass


# ---- Ported from real CPython's Lib/urllib/request.py: OpenerDirector,
# handler chain, Request (fuller version, replaces the minimal one above
# for anything defined after this point in module-global lookup order),
# HTTP auth handlers. See src/modules/urllib_request_extra.py's own doc
# comment (this content was generated from there) for exact scope/gaps.


def _unwrap(url):
    """Transform '<URL:scheme://host/path>' into 'scheme://host/path'."""
    url = str(url).strip()
    if url[:1] == '<' and url[-1:] == '>':
        url = url[1:-1].strip()
    if url[:4] == 'URL:':
        url = url[4:].strip()
    return url


_typeprog = None
def _splittype(url):
    """splittype('type:opaquestring') --> 'type', 'opaquestring'."""
    global _typeprog
    if _typeprog is None:
        _typeprog = re.compile('([^/:]+):(.*)', re.DOTALL)
    match = _typeprog.match(url)
    if match:
        scheme, data = match.groups()
        return scheme.lower(), data
    return None, url


_hostprog = None
def _splithost(url):
    """splithost('//host[:port]/path') --> 'host[:port]', '/path'."""
    global _hostprog
    if _hostprog is None:
        _hostprog = re.compile('//([^/#?]*)(.*)', re.DOTALL)
    match = _hostprog.match(url)
    if match:
        host_port, path = match.groups()
        if path and path[0] != '/':
            path = '/' + path
        return host_port, path
    return None, url


def _splituser(host):
    """splituser('user[:passwd]@host[:port]') --> 'user[:passwd]', 'host[:port]'."""
    user, delim, host = host.rpartition('@')
    return (user if delim else None), host


def _splitpasswd(user):
    """splitpasswd('user:passwd') -> 'user', 'passwd'."""
    user, delim, passwd = user.partition(':')
    return user, (passwd if delim else None)


_portprog = None
def _splitport(host):
    """splitport('host:port') --> 'host', 'port'."""
    global _portprog
    if _portprog is None:
        _portprog = re.compile('(.*):([0-9]*)', re.DOTALL)
    match = _portprog.fullmatch(host)
    if match:
        host, port = match.groups()
        if port:
            return host, port
    return host, None


def _splittag(url):
    """splittag('/path#tag') --> '/path', 'tag'."""
    path, delim, tag = url.rpartition('#')
    if delim:
        return path, tag
    return url, None


_cut_port_re = re.compile(r":\d+$", re.ASCII)
def request_host(request):
    """Return request-host, as defined by RFC 2965 (lowercased)."""
    url = request.full_url
    host = urlparse(url).netloc
    if host == "":
        host = request.get_header("Host", "")
    host = _cut_port_re.sub("", host, 1)
    return host.lower()


class Request:

    def __init__(self, url, data=None, headers={},
                 origin_req_host=None, unverifiable=False,
                 method=None):
        self.full_url = url
        self.headers = {}
        self.unredirected_hdrs = {}
        self._data = None
        self.data = data
        self._tunnel_host = None
        for key, value in headers.items():
            self.add_header(key, value)
        if origin_req_host is None:
            origin_req_host = request_host(self)
        self.origin_req_host = origin_req_host
        self.unverifiable = unverifiable
        if method:
            self.method = method

    @property
    def full_url(self):
        if self.fragment:
            return '{}#{}'.format(self._full_url, self.fragment)
        return self._full_url

    @full_url.setter
    def full_url(self, url):
        self._full_url = _unwrap(url)
        self._full_url, self.fragment = _splittag(self._full_url)
        self._parse()

    @full_url.deleter
    def full_url(self):
        self._full_url = None
        self.fragment = None
        self.selector = ''

    @property
    def data(self):
        return self._data

    @data.setter
    def data(self, data):
        if data != self._data:
            self._data = data
            if self.has_header("Content-length"):
                self.remove_header("Content-length")

    @data.deleter
    def data(self):
        self.data = None

    def _parse(self):
        self.type, rest = _splittype(self._full_url)
        if self.type is None:
            raise ValueError("unknown url type: %r" % self.full_url)
        self.host, self.selector = _splithost(rest)
        if self.host:
            from urllib.parse import unquote
            self.host = unquote(self.host)

    def get_method(self):
        default_method = "POST" if self.data is not None else "GET"
        return getattr(self, 'method', default_method)

    def get_full_url(self):
        return self.full_url

    def set_proxy(self, host, type):
        if self.type == 'https' and not self._tunnel_host:
            self._tunnel_host = self.host
        else:
            self.type = type
            self.selector = self.full_url
        self.host = host

    def has_proxy(self):
        return self.selector == self.full_url

    def add_header(self, key, val):
        self.headers[key.capitalize()] = val

    def add_unredirected_header(self, key, val):
        self.unredirected_hdrs[key.capitalize()] = val

    def has_header(self, header_name):
        return (header_name in self.headers or
                header_name in self.unredirected_hdrs)

    def get_header(self, header_name, default=None):
        return self.headers.get(
            header_name,
            self.unredirected_hdrs.get(header_name, default))

    def remove_header(self, header_name):
        self.headers.pop(header_name, None)
        self.unredirected_hdrs.pop(header_name, None)

    def header_items(self):
        hdrs = {**self.unredirected_hdrs, **self.headers}
        return list(hdrs.items())


class OpenerDirector:
    def __init__(self):
        client_version = "Python-urllib/rustpython"
        self.addheaders = [('User-agent', client_version)]
        self.handlers = []
        self.handle_open = {}
        self.handle_error = {}
        self.process_response = {}
        self.process_request = {}

    def add_handler(self, handler):
        if not hasattr(handler, "add_parent"):
            raise TypeError("expected BaseHandler instance, got %r" %
                            type(handler))

        added = False
        for meth in dir(handler):
            if meth in ["redirect_request", "do_open", "proxy_open"]:
                continue

            i = meth.find("_")
            protocol = meth[:i]
            condition = meth[i+1:]

            if condition.startswith("error"):
                j = condition.find("_") + i + 1
                kind = meth[j+1:]
                try:
                    kind = int(kind)
                except ValueError:
                    pass
                lookup = self.handle_error.get(protocol, {})
                self.handle_error[protocol] = lookup
            elif condition == "open":
                kind = protocol
                lookup = self.handle_open
            elif condition == "response":
                kind = protocol
                lookup = self.process_response
            elif condition == "request":
                kind = protocol
                lookup = self.process_request
            else:
                continue

            handlers = lookup.setdefault(kind, [])
            if handlers:
                bisect.insort(handlers, handler)
            else:
                handlers.append(handler)
            added = True

        if added:
            bisect.insort(self.handlers, handler)
            handler.add_parent(self)

    def close(self):
        pass

    def _call_chain(self, chain, kind, meth_name, *args):
        handlers = chain.get(kind, ())
        for handler in handlers:
            func = getattr(handler, meth_name)
            result = func(*args)
            if result is not None:
                return result

    def open(self, fullurl, data=None, timeout=None):
        import socket
        if timeout is None:
            timeout = socket._GLOBAL_DEFAULT_TIMEOUT
        if isinstance(fullurl, str):
            req = Request(fullurl, data)
        else:
            req = fullurl
            if data is not None:
                req.data = data

        req.timeout = timeout
        protocol = req.type

        meth_name = protocol + "_request"
        for processor in self.process_request.get(protocol, []):
            meth = getattr(processor, meth_name)
            req = meth(req)

        sys.audit('urllib.Request', req.full_url, req.data, req.headers, req.get_method())
        response = self._open(req, data)

        meth_name = protocol + "_response"
        for processor in self.process_response.get(protocol, []):
            meth = getattr(processor, meth_name)
            response = meth(req, response)

        return response

    def _open(self, req, data=None):
        result = self._call_chain(self.handle_open, 'default',
                                  'default_open', req)
        if result:
            return result

        protocol = req.type
        result = self._call_chain(self.handle_open, protocol, protocol +
                                  '_open', req)
        if result:
            return result

        return self._call_chain(self.handle_open, 'unknown',
                                'unknown_open', req)

    def error(self, proto, *args):
        if proto in ('http', 'https'):
            d = self.handle_error['http']
            proto = args[2]
            meth_name = 'http_error_%s' % proto
            http_err = 1
            orig_args = args
        else:
            d = self.handle_error
            meth_name = proto + '_error'
            http_err = 0
        args = (d, proto, meth_name) + args
        result = self._call_chain(*args)
        if result:
            return result

        if http_err:
            args = (d, 'default', 'http_error_default') + orig_args
            return self._call_chain(*args)


class BaseHandler:
    handler_order = 500

    def add_parent(self, parent):
        self.parent = parent

    def close(self):
        pass

    def __lt__(self, other):
        if not hasattr(other, "handler_order"):
            return True
        return self.handler_order < other.handler_order


def _parse_proxy(proxy):
    """Return (scheme, user, password, host/port) given a URL or an authority."""
    scheme, r_scheme = _splittype(proxy)
    if not r_scheme.startswith("/"):
        scheme = None
        authority = proxy
    else:
        if not r_scheme.startswith("//"):
            raise ValueError("proxy URL with no authority: %r" % proxy)
        if '@' in r_scheme:
            host_separator = r_scheme.find('@')
            end = r_scheme.find("/", host_separator)
        else:
            end = r_scheme.find("/", 2)
        if end == -1:
            end = None
        authority = r_scheme[2:end]
    userinfo, hostport = _splituser(authority)
    if userinfo is not None:
        user, password = _splitpasswd(userinfo)
    else:
        user = password = None
    return scheme, user, password, hostport


class HTTPPasswordMgr:

    def __init__(self):
        self.passwd = {}

    def add_password(self, realm, uri, user, passwd):
        if isinstance(uri, str):
            uri = [uri]
        if realm not in self.passwd:
            self.passwd[realm] = {}
        for default_port in True, False:
            reduced_uri = tuple(
                self.reduce_uri(u, default_port) for u in uri)
            self.passwd[realm][reduced_uri] = (user, passwd)

    def find_user_password(self, realm, authuri):
        domains = self.passwd.get(realm, {})
        for default_port in True, False:
            reduced_authuri = self.reduce_uri(authuri, default_port)
            for uris, authinfo in domains.items():
                for uri in uris:
                    if self.is_suburi(uri, reduced_authuri):
                        return authinfo
        return None, None

    def reduce_uri(self, uri, default_port=True):
        """Accept authority or URI and extract only the authority and path."""
        parts = urlparse(uri)
        if parts.netloc:
            scheme = parts.scheme
            authority = parts.netloc
            path = parts.path or '/'
        else:
            scheme = None
            authority = uri
            path = '/'
        host, port = _splitport(authority)
        if default_port and port is None and scheme is not None:
            dport = {"http": 80, "https": 443}.get(scheme)
            if dport is not None:
                authority = "%s:%d" % (host, dport)
        return authority, path

    def is_suburi(self, base, test):
        if base == test:
            return True
        if base[0] != test[0]:
            return False
        prefix = base[1]
        if prefix[-1:] != '/':
            prefix += '/'
        return test[1].startswith(prefix)


class HTTPPasswordMgrWithDefaultRealm(HTTPPasswordMgr):

    def find_user_password(self, realm, authuri):
        user, password = HTTPPasswordMgr.find_user_password(self, realm,
                                                            authuri)
        if user is not None:
            return user, password
        return HTTPPasswordMgr.find_user_password(self, None, authuri)


class HTTPPasswordMgrWithPriorAuth(HTTPPasswordMgrWithDefaultRealm):

    def __init__(self):
        self.authenticated = {}
        super().__init__()

    def add_password(self, realm, uri, user, passwd, is_authenticated=False):
        self.update_authenticated(uri, is_authenticated)
        if realm is not None:
            super().add_password(None, uri, user, passwd)
        super().add_password(realm, uri, user, passwd)

    def update_authenticated(self, uri, is_authenticated=False):
        if isinstance(uri, str):
            uri = [uri]
        for default_port in True, False:
            for u in uri:
                reduced_uri = self.reduce_uri(u, default_port)
                self.authenticated[reduced_uri] = is_authenticated

    def is_authenticated(self, authuri):
        for default_port in True, False:
            reduced_authuri = self.reduce_uri(authuri, default_port)
            for uri in self.authenticated:
                if self.is_suburi(uri, reduced_authuri):
                    return self.authenticated[uri]


class AbstractBasicAuthHandler:

    rx = re.compile('(?:^|,)'
                    '[ \t]*'
                    '([^ \t,]+)'
                    '[ \t]+'
                    'realm=(["\']?)([^"\']*)\\2',
                    re.I)

    def __init__(self, password_mgr=None):
        if password_mgr is None:
            password_mgr = HTTPPasswordMgr()
        self.passwd = password_mgr
        self.add_password = self.passwd.add_password

    def _parse_realm(self, header):
        found_challenge = False
        for mo in AbstractBasicAuthHandler.rx.finditer(header):
            scheme, quote, realm = mo.groups()
            if quote not in ['"', "'"]:
                import warnings
                warnings.warn("Basic Auth Realm was unquoted",
                              UserWarning, 3)
            yield (scheme, realm)
            found_challenge = True

        if not found_challenge:
            if header:
                scheme = header.split()[0]
            else:
                scheme = ''
            yield (scheme, None)

    def http_error_auth_reqed(self, authreq, host, req, headers):
        headers = headers.get_all(authreq)
        if not headers:
            return

        unsupported = None
        for header in headers:
            for scheme, realm in self._parse_realm(header):
                if scheme.lower() != 'basic':
                    unsupported = scheme
                    continue
                if realm is not None:
                    return self.retry_http_basic_auth(host, req, realm)

        if unsupported is not None:
            raise ValueError("AbstractBasicAuthHandler does not "
                             "support the following scheme: %r"
                             % (scheme,))

    def retry_http_basic_auth(self, host, req, realm):
        import base64
        user, pw = self.passwd.find_user_password(realm, host)
        if pw is not None:
            raw = "%s:%s" % (user, pw)
            auth = "Basic " + base64.b64encode(raw.encode()).decode("ascii")
            if req.get_header(self.auth_header, None) == auth:
                return None
            req.add_unredirected_header(self.auth_header, auth)
            return self.parent.open(req, timeout=req.timeout)
        else:
            return None

    def http_request(self, req):
        import base64
        if (not hasattr(self.passwd, 'is_authenticated') or
           not self.passwd.is_authenticated(req.full_url)):
            return req

        if not req.has_header('Authorization'):
            user, passwd = self.passwd.find_user_password(None, req.full_url)
            credentials = '{0}:{1}'.format(user, passwd).encode()
            auth_str = base64.standard_b64encode(credentials).decode()
            req.add_unredirected_header('Authorization',
                                        'Basic {}'.format(auth_str.strip()))
        return req

    def http_response(self, req, response):
        if hasattr(self.passwd, 'is_authenticated'):
            if 200 <= response.code < 300:
                self.passwd.update_authenticated(req.full_url, True)
            else:
                self.passwd.update_authenticated(req.full_url, False)
        return response

    https_request = http_request
    https_response = http_response


class HTTPBasicAuthHandler(AbstractBasicAuthHandler, BaseHandler):

    auth_header = 'Authorization'

    def http_error_401(self, req, fp, code, msg, headers):
        url = req.full_url
        response = self.http_error_auth_reqed('www-authenticate',
                                          url, req, headers)
        return response


def parse_keqv_list(l):
    """Parse list of key=value strings where keys are not duplicated."""
    parsed = {}
    for elt in l:
        k, v = elt.split('=', 1)
        if v[0] == '"' and v[-1] == '"':
            v = v[1:-1]
        parsed[k] = v
    return parsed


def parse_http_list(s):
    """Parse lists as described by RFC 2068 Section 2."""
    res = []
    part = ''

    escape = quote = False
    for cur in s:
        if escape:
            part += cur
            escape = False
            continue
        if quote:
            if cur == '\\':
                escape = True
                continue
            elif cur == '"':
                quote = False
            part += cur
            continue

        if cur == ',':
            res.append(part)
            part = ''
            continue

        if cur == '"':
            quote = True

        part += cur

    if part:
        res.append(part)

    return [part.strip() for part in res]


# Return n random bytes.
_randombytes = os.urandom


class AbstractDigestAuthHandler:
    # Digest authentication is specified in RFC 2617/7616.

    def __init__(self, passwd=None):
        if passwd is None:
            passwd = HTTPPasswordMgr()
        self.passwd = passwd
        self.add_password = self.passwd.add_password
        self.retried = 0
        self.nonce_count = 0
        self.last_nonce = None

    def reset_retry_count(self):
        self.retried = 0

    def http_error_auth_reqed(self, auth_header, host, req, headers):
        authreq = headers.get(auth_header, None)
        if self.retried > 5:
            raise HTTPError(req.full_url, 401, "digest auth failed",
                            headers, None)
        else:
            self.retried += 1
        if authreq:
            scheme = authreq.split()[0]
            if scheme.lower() == 'digest':
                return self.retry_http_digest_auth(req, authreq)
            elif scheme.lower() != 'basic':
                raise ValueError("AbstractDigestAuthHandler does not support"
                                 " the following scheme: '%s'" % scheme)

    def retry_http_digest_auth(self, req, auth):
        token, challenge = auth.split(' ', 1)
        chal = parse_keqv_list(filter(None, parse_http_list(challenge)))
        auth = self.get_authorization(req, chal)
        if auth:
            auth_val = 'Digest %s' % auth
            if req.headers.get(self.auth_header, None) == auth_val:
                return None
            req.add_unredirected_header(self.auth_header, auth_val)
            resp = self.parent.open(req, timeout=req.timeout)
            return resp

    def get_cnonce(self, nonce):
        s = "%s:%s:%s:" % (self.nonce_count, nonce, time.ctime())
        b = s.encode("ascii") + _randombytes(8)
        dig = hashlib.sha1(b).hexdigest()
        return dig[:16]

    def get_authorization(self, req, chal):
        try:
            realm = chal['realm']
            nonce = chal['nonce']
            qop = chal.get('qop')
            algorithm = chal.get('algorithm', 'MD5')
            opaque = chal.get('opaque', None)
        except KeyError:
            return None

        H, KD = self.get_algorithm_impls(algorithm)
        if H is None:
            return None

        user, pw = self.passwd.find_user_password(realm, req.full_url)
        if user is None:
            return None

        if req.data is not None:
            entdig = self.get_entity_digest(req.data, chal)
        else:
            entdig = None

        A1 = "%s:%s:%s" % (user, realm, pw)
        A2 = "%s:%s" % (req.get_method(), req.selector)
        if qop is None:
            respdig = KD(H(A1), "%s:%s" % (nonce, H(A2)))
        elif 'auth' in qop.split(','):
            if nonce == self.last_nonce:
                self.nonce_count += 1
            else:
                self.nonce_count = 1
                self.last_nonce = nonce
            ncvalue = '%08x' % self.nonce_count
            cnonce = self.get_cnonce(nonce)
            noncebit = "%s:%s:%s:%s:%s" % (nonce, ncvalue, cnonce, 'auth', H(A2))
            respdig = KD(H(A1), noncebit)
        else:
            raise URLError("qop '%s' is not supported." % qop)

        base = 'username="%s", realm="%s", nonce="%s", uri="%s", ' \
               'response="%s"' % (user, realm, nonce, req.selector,
                                  respdig)
        if opaque:
            base += ', opaque="%s"' % opaque
        if entdig:
            base += ', digest="%s"' % entdig
        base += ', algorithm="%s"' % algorithm
        if qop:
            base += ', qop=auth, nc=%s, cnonce="%s"' % (ncvalue, cnonce)
        return base

    def get_algorithm_impls(self, algorithm):
        if algorithm == 'MD5':
            H = lambda x: hashlib.md5(x.encode("ascii")).hexdigest()
        elif algorithm == 'SHA':
            H = lambda x: hashlib.sha1(x.encode("ascii")).hexdigest()
        elif algorithm == 'SHA-256':
            H = lambda x: hashlib.sha256(x.encode("ascii")).hexdigest()
        else:
            raise ValueError("Unsupported digest authentication "
                             "algorithm %r" % algorithm)
        KD = lambda s, d: H("%s:%s" % (s, d))
        return H, KD

    def get_entity_digest(self, data, chal):
        return None


def _proxy_bypass_macosx_sysconf(host, proxy_settings):
    """
    Return True iff this host shouldn't be accessed using a proxy

    proxy_settings come from _scproxy._get_proxy_settings or get mocked ie:
    { 'exclude_simple': bool,
      'exceptions': ['foo.bar', '*.bar.com', '127.0.0.1', '10.1', '10.0/16'] }
    """
    from fnmatch import fnmatch
    from ipaddress import AddressValueError, IPv4Address

    hostonly, port = _splitport(host)

    def ip2num(ipAddr):
        parts = ipAddr.split('.')
        parts = list(map(int, parts))
        if len(parts) != 4:
            parts = (parts + [0, 0, 0, 0])[:4]
        return (parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]

    if '.' not in host:
        if proxy_settings['exclude_simple']:
            return True

    hostIP = None
    try:
        hostIP = int(IPv4Address(hostonly))
    except AddressValueError:
        pass

    for value in proxy_settings.get('exceptions', ()):
        if not value:
            continue

        m = re.match(r"(\d+(?:\.\d+)*)(/\d+)?", value)
        if m is not None and hostIP is not None:
            base = ip2num(m.group(1))
            mask = m.group(2)
            if mask is None:
                mask = 8 * (m.group(1).count('.') + 1)
            else:
                mask = int(mask[1:])

            if mask < 0 or mask > 32:
                continue

            mask = 32 - mask

            if (hostIP >> mask) == (base >> mask):
                return True

        elif fnmatch(host, value):
            return True

    return False


def _proxy_bypass_winreg_override(host, override):
    """Return True if the host should bypass the proxy server.

    The proxy override list is obtained from the Windows Internet
    settings proxy override registry value, e.g.:
    "www.example.com;*.example.net; 192.168.0.1"
    """
    from fnmatch import fnmatch

    host, _ = _splitport(host)
    proxy_override = override.split(';')
    for test in proxy_override:
        test = test.strip()
        if test == '<local>':
            if '.' not in host:
                return True
        elif fnmatch(host, test):
            return True
    return False
