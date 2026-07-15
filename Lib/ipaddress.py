"""Lightweight IP address manipulation library for RustPython.

Minimal implementation that avoids CPython's ~200MB module-level allocations.
Uses lazy property evaluation instead of pre-computed constant sets.
"""

def _ipv4_to_int(parts):
    return (int(parts[0]) << 24) | (int(parts[1]) << 16) | (int(parts[2]) << 8) | int(parts[3])

def _int_to_ipv4(ip):
    return '.'.join(str((ip >> (8*i)) & 0xFF) for i in range(3, -1, -1))

def _is_ipv4_private(ip):
    return ((ip & 0xFF000000) == 0x0A000000 or
            (ip & 0xFFF00000) == 0xAC100000 or
            (ip & 0xFFFF0000) == 0xC0A80000)

def _is_ipv4_loopback(ip):
    return (ip & 0xFF000000) == 0x7F000000

def _is_ipv4_multicast(ip):
    return (ip & 0xF0000000) == 0xE0000000

def _is_ipv4_link_local(ip):
    return (ip & 0xFFFF0000) == 0xA9FE0000

class IPv4Address:
    def __init__(self, address):
        if isinstance(address, int):
            self._ip = address & 0xFFFFFFFF
        elif isinstance(address, IPv4Address):
            self._ip = address._ip
        else:
            s = str(address)
            parts = s.split('.')
            if len(parts) != 4:
                raise ValueError("Invalid IPv4 address")
            self._ip = _ipv4_to_int(parts)

    def __str__(self):
        return _int_to_ipv4(self._ip)

    def __repr__(self):
        return "IPv4Address('%s')" % str(self)

    def __eq__(self, other):
        if isinstance(other, IPv4Address):
            return self._ip == other._ip
        return NotImplemented

    def __hash__(self):
        return self._ip

    def __int__(self):
        return self._ip

    def __contains__(self, other):
        if isinstance(other, IPv4Address):
            return self._ip == other._ip
        return False

    @property
    def packed(self):
        import struct
        return struct.pack('!I', self._ip)

    @property
    def version(self):
        return 4

    @property
    def max_prefixlen(self):
        return 32

    @property
    def is_private(self):
        return _is_ipv4_private(self._ip)

    @property
    def is_global(self):
        return not (self.is_private or self.is_loopback or self.is_multicast or self.is_link_local)

    @property
    def is_multicast(self):
        return _is_ipv4_multicast(self._ip)

    @property
    def is_loopback(self):
        return _is_ipv4_loopback(self._ip)

    @property
    def is_link_local(self):
        return _is_ipv4_link_local(self._ip)


def _ipv6_to_int(parts):
    """Convert expanded IPv6 parts to int."""
    result = 0
    for p in parts:
        result = (result << 16) | int(p, 16)
    return result

def _int_to_ipv6(ip):
    """Convert int to IPv6 string with :: compression."""
    hextets = []
    for i in range(8):
        hextets.append('%x' % ((ip >> (112 - (16 * i))) & 0xFFFF))
    # Find longest run of zeros for :: compression
    best_start = -1
    best_len = 0
    cur_start = -1
    cur_len = 0
    for i, h in enumerate(hextets):
        if h == '0':
            if cur_start == -1:
                cur_start = i
                cur_len = 1
            else:
                cur_len += 1
            if cur_len > best_len:
                best_start = cur_start
                best_len = cur_len
        else:
            cur_start = -1
            cur_len = 0
    if best_len >= 2:
        before = ':'.join(hextets[:best_start])
        after = ':'.join(hextets[best_start+best_len:])
        return before + '::' + after
    return ':'.join(hextets)

def _is_ipv6_private(ip):
    return (ip >> 120) & 0xFF == 0xFC or (ip >> 120) & 0xFF == 0xFD

def _is_ipv6_loopback(ip):
    return ip == 1

def _is_ipv6_multicast(ip):
    return (ip >> 120) & 0xFF == 0xFF

def _is_ipv6_link_local(ip):
    return (ip >> 118) & 0x3FF == 0x3FE  # fe80::/10

class IPv6Address:
    def __init__(self, address):
        if isinstance(address, int):
            self._ip = address & ((1 << 128) - 1)
        elif isinstance(address, IPv6Address):
            self._ip = address._ip
        else:
            s = str(address)
            if s == '::':
                self._ip = 0
            elif s.startswith('::'):
                self._ip = _ipv6_to_int(['0'] * (8 - len(s[2:].split(':'))) + s[2:].split(':'))
            elif s.endswith('::'):
                self._ip = _ipv6_to_int(s[:-2].split(':') + ['0'] * (8 - len(s[:-2].split(':'))))
            elif '::' in s:
                parts = s.split('::')
                left = parts[0].split(':') if parts[0] else []
                right = parts[1].split(':') if parts[1] else []
                self._ip = _ipv6_to_int(left + ['0'] * (8 - len(left) - len(right)) + right)
            else:
                self._ip = _ipv6_to_int(s.split(':'))

    def __str__(self):
        return _int_to_ipv6(self._ip)

    def __repr__(self):
        return "IPv6Address('%s')" % str(self)

    def __eq__(self, other):
        if isinstance(other, IPv6Address):
            return self._ip == other._ip
        return NotImplemented

    def __hash__(self):
        return self._ip

    def __int__(self):
        return self._ip

    @property
    def version(self):
        return 6

    @property
    def max_prefixlen(self):
        return 128

    @property
    def is_private(self):
        return _is_ipv6_private(self._ip)

    @property
    def is_global(self):
        return not (self.is_private or self.is_loopback or self.is_multicast or self.is_link_local)

    @property
    def is_multicast(self):
        return _is_ipv6_multicast(self._ip)

    @property
    def is_loopback(self):
        return _is_ipv6_loopback(self._ip)

    @property
    def is_link_local(self):
        return _is_ipv6_link_local(self._ip)


def ip_address(address):
    """Factory: detect IPv4 or IPv6 from string."""
    if isinstance(address, IPv4Address):
        return address
    if isinstance(address, IPv6Address):
        return address
    s = str(address)
    if '.' in s and s.count(':') == 0:
        return IPv4Address(s)
    return IPv6Address(s)


class _BaseNetwork:
    def __init__(self, address, strict=True):
        if '/' in str(address):
            addr_str, self._prefixlen = str(address).split('/')
            self._prefixlen = int(self._prefixlen)
        else:
            addr_str = str(address)
            self._prefixlen = self._max_prefixlen
        self._network = int(self._address_class(addr_str))
        self._mask = ((1 << self._prefixlen) - 1) << (self._max_prefixlen - self._prefixlen)
        self._network = self._network & self._mask

    def __str__(self):
        return '%s/%d' % (str(self._address_class(self._network)), self._prefixlen)

    def __contains__(self, other):
        if isinstance(other, str):
            other = self._address_class(other)
        if isinstance(other, self._address_class):
            return (int(other) & self._mask) == self._network
        return NotImplemented

    @property
    def network_address(self):
        return self._address_class(self._network)

    @property
    def netmask(self):
        return self._address_class(self._mask)

    @property
    def is_private(self):
        return self.network_address.is_private

    @property
    def is_global(self):
        return self.network_address.is_global


class IPv4Network(_BaseNetwork):
    _address_class = IPv4Address
    _max_prefixlen = 32

    def __init__(self, address, strict=True):
        _BaseNetwork.__init__(self, address, strict)


class IPv6Network(_BaseNetwork):
    _address_class = IPv6Address
    _max_prefixlen = 128

    def __init__(self, address, strict=True):
        _BaseNetwork.__init__(self, address, strict)


def ip_network(address, strict=True):
    if isinstance(address, IPv4Network):
        return address
    if isinstance(address, IPv6Network):
        return address
    s = str(address)
    if '.' in s:
        return IPv4Network(s, strict)
    return IPv6Network(s, strict)


def ip_interface(address):
    if isinstance(address, IPv4Address) or isinstance(address, IPv4Network):
        return IPv4Interface(address) if not isinstance(address, IPv4Interface) else address
    return IPv6Interface(address) if not isinstance(address, IPv6Interface) else address


class IPv4Interface:
    def __init__(self, address):
        if isinstance(address, IPv4Address):
            self._ip = address
            self._network = IPv4Network(address)
        elif isinstance(address, IPv4Network):
            self._network = address
            self._ip = address.network_address
        elif '/' in str(address):
            addr_str, prefix = str(address).split('/')
            self._ip = IPv4Address(addr_str)
            self._network = IPv4Network(address)
        else:
            self._ip = IPv4Address(address)
            self._network = IPv4Network(str(address) + '/32')

    def __str__(self):
        return '%s/%d' % (str(self._ip), self._network._prefixlen)

    @property
    def ip(self):
        return self._ip

    @property
    def network(self):
        return self._network


class IPv6Interface:
    def __init__(self, address):
        if isinstance(address, IPv6Address):
            self._ip = address
            self._network = IPv6Network(address)
        elif isinstance(address, IPv6Network):
            self._network = address
            self._ip = address.network_address
        elif '/' in str(address):
            self._ip = IPv6Address(str(address).split('/')[0])
            self._network = IPv6Network(address)
        else:
            self._ip = IPv6Address(address)
            self._network = IPv6Network(str(address) + '/128')

    def __str__(self):
        return '%s/%d' % (str(self._ip), self._network._prefixlen)

    @property
    def ip(self):
        return self._ip

    @property
    def network(self):
        return self._network
