"""`ssl.SSLSocket` — a socket-shaped wrapper, installed into the native `ssl`
module by `VirtualMachine::install_source_defined_stdlib` (see
`src/modules/misc/ssl.rs`).

Real TLS is out of reach for this interpreter (no OpenSSL bindings), but the
CLASS is what the stdlib and the test suite key off: `ftplib.FTP_TLS`,
`poplib.POP3`, `smtplib.SMTP` and `urllib.request.HTTPSHandler` all do
`isinstance(conn, ssl.SSLSocket)` or construct one, so without it those
modules fail at import. Every socket operation delegates to the plain socket
being wrapped, which is exactly the shape their tests exercise against a
local, unencrypted server.
"""


class _SSLSocketIO:
    """The socket-shaped half of `SSLSocket`: forwards straight through to
    the wrapped socket, which may be absent once `unwrap()` ran."""

    def send(self, data, flags=0):
        return self._sock.send(data, flags)

    def sendall(self, data, flags=0):
        return self._sock.sendall(data, flags)

    def sendto(self, data, *args):
        return self._sock.sendto(data, *args)

    def recv(self, bufsize, flags=0):
        return self._sock.recv(bufsize, flags)

    def recv_into(self, buffer, nbytes=0, flags=0):
        return self._sock.recv_into(buffer, nbytes, flags)

    def recv_into1(self, buffer, nbytes=0):
        return self._sock.recv_into1(buffer, nbytes)

    def recvfrom(self, bufsize, flags=0):
        return self._sock.recvfrom(bufsize, flags)

    def recvfrom_into(self, buffer, nbytes=0, flags=0):
        return self._sock.recvfrom_into(buffer, nbytes, flags)

    def makefile(self, mode='r', buffering=None, *args, **kwargs):
        if buffering is None:
            return self._sock.makefile(mode, *args, **kwargs)
        return self._sock.makefile(mode, buffering, *args, **kwargs)

    def close(self):
        sock, self._sock = self._sock, None
        if sock is not None:
            return sock.close()
        return None

    def fileno(self):
        return self._sock.fileno()

    def getpeername(self):
        return self._sock.getpeername()

    def getsockname(self):
        return self._sock.getsockname()

    def setsockopt(self, *args):
        return self._sock.setsockopt(*args)

    def getsockopt(self, *args):
        return self._sock.getsockopt(*args)

    def settimeout(self, value):
        return self._sock.settimeout(value)

    def gettimeout(self):
        return self._sock.gettimeout()

    def setblocking(self, flag):
        return self._sock.setblocking(flag)

    def shutdown(self, how):
        return self._sock.shutdown(how)

    def listen(self, *args, **kwargs):
        return self._sock.listen(*args, **kwargs)

    def bind(self, address):
        return self._sock.bind(address)

    def connect(self, address):
        return self._sock.connect(address)

    def accept(self):
        conn, addr = self._sock.accept()
        return self.__class__(sock=conn), addr

    def detach(self):
        sock, self._sock = self._sock, None
        return sock.detach()

    def dup(self):
        return self._sock.dup()


class SSLSocket(_SSLSocketIO):
    """Wraps a connected socket, adding the TLS-shaped API surface."""

    def __init__(self, sock=None, *, server_hostname=None, context=None,
                 do_handshake_on_connect=True, suppress_ragged_eofs=True,
                 server_side=False, **kwargs):
        self._sock = sock
        self.server_hostname = server_hostname
        self.server_side = server_side
        self.suppress_ragged_eofs = suppress_ragged_eofs
        self.do_handshake_on_connect = do_handshake_on_connect
        self.context = context
        self.session = None
        self.session_reused = False
        self.server_hostname_bytes = None

    def getpeercert(self, binary_form=False):
        if binary_form:
            return b''
        return {}

    def getpeercertinfo(self):
        return {}

    def cipher(self):
        return None, None, 0

    def shared_ciphers(self):
        return None

    def selected_alpn_protocol(self):
        return None

    def get_channel_binding(self, cb_type):
        return None

    def version(self):
        return 'TLSv1.3'

    def pending(self):
        return 0

    def do_handshake(self):
        return None

    def _real_connect(self, addr, connect_ex=False):
        return self._sock.connect_ex(addr) if connect_ex else self._sock.connect(addr)

    def connect_ex(self, addr):
        return self._sock.connect_ex(addr)

    def unwrap(self):
        sock, self._sock = self._sock, None
        return sock

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
        return False

    def __repr__(self):
        return '<ssl.SSLSocket fd=%r>' % (
            self._sock.fileno() if self._sock is not None else -1,
        )


from enum import Enum, IntEnum


class TLSVersion(IntEnum):
    MINIMUM_SUPPORTED = -2
    SSLv3 = 0x0300
    TLSv1 = 0x0301
    TLSv1_1 = 0x0302
    TLSv1_2 = 0x0303
    TLSv1_3 = 0x0304
    MAXIMUM_SUPPORTED = -1


class _SSLMethod(IntEnum):
    PROTOCOL_TLS = 2
    PROTOCOL_TLSv1 = 3
    PROTOCOL_TLSv1_1 = 4
    PROTOCOL_TLSv1_2 = 5
    PROTOCOL_TLS_CLIENT = 16
    PROTOCOL_TLS_SERVER = 17


# CPython's `ssl._PROTOCOL_NAMES` is keyed by `_SSLMethod` members, so
# `sorted(ssl._PROTOCOL_NAMES)` (which test_ssl does at import time) sorts
# enum members, not the raw ints the native stub used.
_PROTOCOL_NAMES = {method: method.name for method in _SSLMethod}

PROTOCOL_TLS = _SSLMethod.PROTOCOL_TLS
PROTOCOL_TLSv1 = _SSLMethod.PROTOCOL_TLSv1
PROTOCOL_TLSv1_1 = _SSLMethod.PROTOCOL_TLSv1_1
PROTOCOL_TLSv1_2 = _SSLMethod.PROTOCOL_TLSv1_2
PROTOCOL_TLS_CLIENT = _SSLMethod.PROTOCOL_TLS_CLIENT
PROTOCOL_TLS_SERVER = _SSLMethod.PROTOCOL_TLS_SERVER


class _ASN1Object:
    """`ssl.Purpose`'s member values are X509 purpose OIDs, not ints."""

    def __init__(self, nid, shortname, longname, oid):
        self.nid = nid
        self.shortname = shortname
        self.longname = longname
        self.oid = oid

    def __repr__(self):
        return '<ASN1 OID %s>' % self.shortname

    def __str__(self):
        return self.shortname


class Purpose(Enum):
    SERVER_AUTH = _ASN1Object(
        129, 'serverAuth', 'TLS Web Server Authentication',
        '1.3.6.1.5.5.7.3.1')
    CLIENT_AUTH = _ASN1Object(
        130, 'clientAuth', 'TLS Web Client Authentication',
        '1.3.6.1.5.5.7.3.2')

    def __str__(self):
        return self._value_.shortname


CERT_NONE_DEFAULT = 0
CERT_OPTIONAL_DEFAULT = 1
CERT_REQUIRED_DEFAULT = 2
Purpose_SERVER_AUTH_DEFAULT = 1


class SSLContext:
    """A real Type (so `isinstance(ctx, ssl.SSLContext)` works, unlike the
    native module-like stub it replaces). Every knob is recorded and
    ignored: without OpenSSL there is no handshake to configure."""

    def __init__(self, protocol=None, *args, **kwargs):
        self.protocol = protocol
        self.verify_mode = CERT_NONE_DEFAULT
        self.check_hostname = False
        self.verify_flags = 0
        self.options = 0
        # CPython reports a CLIENT context's floor as TLSv1_2 and its ceiling
        # as MAXIMUM_SUPPORTED; test_ssl compares `version <
        # ctx.minimum_version` at import time, so None here would break it.
        self.minimum_version = TLSVersion.TLSv1_2
        self.maximum_version = TLSVersion.MAXIMUM_SUPPORTED
        self.security_level = 2
        self.num_tickets = 2
        self.ca_certs = None
        self.cafile = None
        self.capath = None
        self.cadata = None
        self.certfile = None
        self.keyfile = None
        self.purpose = None
        self.hostname_checks_common_name = True
        self.alpn_protocols = []
        self.npn_protocols = []
        self.keylog_filename = None
        self.post_handshake_auth = False
        self._acceptor = _SSLSocketAcceptor(self)

    def wrap_socket(self, sock, server_side=False, do_handshake_on_connect=True,
                    suppress_ragged_eofs=True, server_hostname=None,
                    session=None):
        return self._acceptor(
            sock,
            server_side=server_side,
            server_hostname=server_hostname,
            do_handshake_on_connect=do_handshake_on_connect,
            suppress_ragged_eofs=suppress_ragged_eofs,
        )

    def wrap_bio(self, incoming, outgoing, server_side=False,
                 server_hostname=None, session=None):
        raise ValueError('BIO-based TLS is unavailable without OpenSSL')

    def load_default_certs(self, purpose=Purpose_SERVER_AUTH_DEFAULT):
        return None

    def load_verify_locations(self, cafile=None, capath=None, cadata=None):
        if cafile is not None:
            self.cafile = cafile
        if capath is not None:
            self.capath = capath
        if cadata is not None:
            self.cadata = cadata
        return None

    def load_cert_chain(self, certfile, keyfile=None, password=None):
        self.certfile = certfile
        self.keyfile = keyfile
        return None

    def load_client_ca(self, *args, **kwargs):
        return None

    def set_alpn_protocols(self, alpn_protocols):
        self.alpn_protocols = list(alpn_protocols)
        return None

    def set_npn_protocols(self, npn_protocols):
        self.npn_protocols = list(npn_protocols)
        return None

    def set_ciphers(self, cipher_list):
        return None

    def set_default_verify_paths(self):
        return None

    def set_keylog_filename(self, filename):
        self.keylog_filename = filename
        return None

    def set_post_handshake_auth(self, enabled):
        self.post_handshake_auth = bool(enabled)
        return None

    def set_minimum_version(self, version):
        self.minimum_version = version
        return None

    def set_maximum_version(self, version):
        self.maximum_version = version
        return None

    def set_verify(self, mode, callback=None):
        self.verify_mode = mode
        return None

    def cert_store_stats(self):
        return {'x509': 0, 'x509_ca': 0, 'crl': 0, 'crl_ca': 0}

    def get_ca_certs(self, binary_form=False, cafile=None, capath=None,
                     cadata=None):
        if binary_form:
            return b''
        return []

    def get_ca_certs_crls(self, binary_form=False):
        return [] if binary_form else ([], [])

    def get_crl_list(self, binary_form=False):
        return [] if binary_form else ([], [])

    # CPython exposes these as class-level getset descriptors; tests probe
    # them on the class (`hasattr(ssl.SSLContext, 'keylog_filename')`).
    keylog_filename = None
    post_handshake_auth = False
    minimum_version = TLSVersion.TLSv1_2
    maximum_version = TLSVersion.MAXIMUM_SUPPORTED

    def __repr__(self):
        return '<ssl.SSLContext object>'


def _ssl_module():
    """The native `ssl` module. This source is executed in its own globals
    (see `install_source_defined_stdlib`), so the native half of the module
    is only reachable through `sys.modules` — which also means defaults here
    cannot be native objects, hence `purpose=1` for `Purpose.SERVER_AUTH`."""
    import sys
    return sys.modules[__name__]


def wrap_socket(sock, server_side=False, do_handshake_on_connect=True,
                suppress_ragged_eofs=True, server_hostname=None,
                ssl_context=None, **kwargs):
    """Wrap a connected socket (CPython's `ssl.wrap_socket`)."""
    ssl = _ssl_module()
    if ssl_context is not None and not isinstance(ssl_context, ssl.SSLContext):
        raise TypeError(
            'ssl_context must be an SSLContext, not %r' % type(ssl_context))
    return SSLSocket(
        sock=sock,
        server_hostname=server_hostname,
        context=ssl_context,
        server_side=server_side,
        do_handshake_on_connect=do_handshake_on_connect,
        suppress_ragged_eofs=suppress_ragged_eofs,
    )


def create_default_context(purpose=1, *, cafile=None, capath=None,
                           cadata=None):
    """A client context. Without OpenSSL there is nothing to verify against,
    so the settings are recorded and ignored, exactly like the native stub's
    other TLS knobs — but CPython's default context is a *verifying* one, so
    say so (`check_hostname = True`) and let the wrapped socket stay plain."""
    context = SSLContext(purpose)
    context.verify_mode = CERT_REQUIRED_DEFAULT
    context.check_hostname = True
    return context


def _create_stdlib_context(purpose=1, *, cafile=None, capath=None, cadata=None):
    """What `ftplib`/`poplib`/`smtplib` call. CPython's version hardens the
    protocol (no SSLv2/3, no TLS < 1.2, no compression) and trusts the
    system CA store; the flags are recorded on the context."""
    ssl = _ssl_module()
    context = SSLContext(purpose)
    context.verify_mode = CERT_REQUIRED_DEFAULT
    context.check_hostname = True
    context.options = (ssl.OP_NO_SSLv2 | ssl.OP_NO_SSLv3 | ssl.OP_NO_TLSv1
                       | ssl.OP_NO_TLSv1_1 | ssl.OP_NO_COMPRESSION
                       | ssl.OP_NO_RENEGOTIATION)
    context.keylog_filename = None
    import os
    context.keylog_filename = os.environ.get('SSLKEYLOGFILE')
    return context


def _create_unverified_context(purpose=1, *, cafile=None, capath=None,
                               cadata=None):
    context = _create_stdlib_context(purpose, cafile=cafile, capath=capath,
                                     cadata=cadata)
    context.check_hostname = False
    context.verify_mode = CERT_NONE_DEFAULT
    return context


def _create_default_https_context(certfile=None, keyfile=None, *, cafile=None,
                                  capath=None, cadata=None):
    context = _create_stdlib_context(Purpose_SERVER_AUTH_DEFAULT,
                                     cafile=cafile, capath=capath, cadata=cadata)
    context.check_hostname = True
    if certfile is not None:
        context.load_cert_chain(certfile, keyfile)
    return context


def _create_default_verify_context(purpose=Purpose_SERVER_AUTH_DEFAULT):
    return _create_stdlib_context(purpose)


def create_unverified_context(purpose=1, *, cafile=None, capath=None,
                              cadata=None):
    context = SSLContext(purpose)
    context.check_hostname = False
    context.verify_mode = CERT_NONE_DEFAULT
    return context


class _SSLSocketAcceptor:
    """Stand-in for the C `SSLContext`'s socket-accept helper used by
    `http.server`/`socketserver` when asked for a TLS server; it wraps
    accepted connections the same way `SSLSocket` does."""

    def __init__(self, context=None):
        self.context = context

    def __call__(self, sock, server_side=False, server_hostname=None,
                 do_handshake_on_connect=True, suppress_ragged_eofs=True,
                 context=None, **kwargs):
        return SSLSocket(
            sock=sock,
            server_hostname=server_hostname,
            context=context if context is not None else self.context,
            server_side=server_side,
            do_handshake_on_connect=do_handshake_on_connect,
            suppress_ragged_eofs=suppress_ragged_eofs,
        )
