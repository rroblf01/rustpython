"""
tarfile.py — Tar file archive support.

Provides a basic interface for reading and writing tar archives.
Delegates to the native RustPython tarfile module when available.
"""
import sys

# Try to use the native tarfile module
_tarfile_native = sys.modules.get("tarfile")


# ── Exceptions ──────────────────────────────────────────────────────────────

class TarError(Exception):
    """Base for tarfile errors."""
    pass


class ReadError(TarError):
    """Raised when a tar archive cannot be read."""
    pass


class CompressionError(TarError):
    """Raised when compression is not supported."""
    pass


# ── Constants ───────────────────────────────────────────────────────────────

RECORDSIZE = 512
NAMSIZE = 100
TUNMLEN = 32
TGNMLEN = 32

# Supported tar formats
USTAR_FORMAT = 0
GNU_FORMAT = 1
PAX_FORMAT = 2
DEFAULT_FORMAT = GNU_FORMAT

# Type flags
REGTYPE = "0"
AREGTYPE = "\0"
LNKTYPE = "1"
SYMTYPE = "2"
CHRTYPE = "3"
BLKTYPE = "4"
DIRTYPE = "5"
FIFOTYPE = "6"
CONTTYPE = "7"

# Open modes
OPEN_READ = "r"
OPEN_WRITE = "w"
OPEN_APPEND = "a"


class TarInfo:
    """Information about a file in the tar archive."""

    def __init__(self, name=""):
        self.name = name
        self.size = 0
        self.mtime = 0
        self.mode = 0o644
        self.type = REGTYPE
        self.linkname = ""
        self.uid = 0
        self.gid = 0
        self.uname = ""
        self.gname = ""
        self.devmajor = 0
        self.devminor = 0
        self.offset = 0
        self._chksum = 0

    @classmethod
    def from_header(cls, header):
        info = cls.__new__(cls)
        # Parse basic header fields
        info.name = header[:100].rstrip(b"\0").decode("utf-8", errors="replace")
        size_bytes = header[124:136]
        info.size = int(size_bytes.split(b"\0")[0].strip(), 8) if size_bytes.strip() else 0
        info.type = header[156:157].decode("ascii", errors="replace") or REGTYPE
        return info

    def __repr__(self):
        return f"<TarInfo {self.name!r}>"


class TarFile:
    """Tar archive reader/writer."""

    def __init__(self, name, mode="r", fileobj=None, format=DEFAULT_FORMAT):
        self.name = name
        self.mode = mode
        self.fileobj = fileobj
        self.format = format
        self._members = []
        self._closed = False

        if mode == "r":
            self._handle = fileobj or open(name, "rb")
            self._read_headers()
        elif mode in ("w", "x"):
            self._handle = fileobj or open(name, "wb")
        elif mode == "a":
            self._handle = fileobj or open(name, "r+b")
            self._read_headers()
        else:
            raise ValueError(f"invalid mode {mode!r}")

    def _read_headers(self):
        """Read all tar headers and build member list."""
        while True:
            header = self._handle.read(RECORDSIZE)
            if len(header) < RECORDSIZE:
                break
            # Check for end-of-archive (zero blocks)
            if header == b"\0" * RECORDSIZE:
                # Skip the second zero block
                self._handle.read(RECORDSIZE)
                break
            info = TarInfo.from_header(header)
            if info.name:
                self._members.append(info)
            # Skip to next header
            data_blocks = (info.size + RECORDSIZE - 1) // RECORDSIZE if info.size > 0 else 0
            self._handle.seek(data_blocks * RECORDSIZE, 1)

    def getmembers(self):
        return self._members

    def getnames(self):
        return [m.name for m in self._members]

    def extractall(self, path=".", members=None, *, numeric_owner=False):
        import os
        for m in (members or self._members):
            target = os.path.join(path, m.name)
            if m.type == DIRTYPE:
                os.makedirs(target, exist_ok=True)
            else:
                target_dir = os.path.dirname(target)
                if target_dir:
                    os.makedirs(target_dir, exist_ok=True)
                if m.size > 0:
                    # Read file data
                    with open(target, "wb") as f:
                        remaining = m.size
                        while remaining > 0:
                            chunk = self._handle.read(min(remaining, 65536))
                            if not chunk:
                                break
                            f.write(chunk)
                            remaining -= len(chunk)

    def extract(self, member, path="", *, numeric_owner=False):
        if isinstance(member, str):
            name = member
            for m in self._members:
                if m.name == name:
                    member = m
                    break
            else:
                raise KeyError(f"member {name!r} not found")
        self.extractall(path=path, members=[member])

    def add(self, name, arcname=None, recursive=True, *, filter=None):
        import os, stat as statmod
        arcname = arcname or name
        info = TarInfo(arcname)
        st = os.stat(name)
        info.size = st.st_size
        info.mtime = int(st.st_mtime)
        info.mode = st.st_mode
        if statmod.S_ISDIR(st.st_mode):
            info.type = DIRTYPE
            info.size = 0
        self._members.append(info)
        # Write header
        self._write_header(info)
        # Write data for regular files
        if info.type == REGTYPE:
            with open(name, "rb") as f:
                while True:
                    chunk = f.read(65536)
                    if not chunk:
                        break
                    self._handle.write(chunk)
            # Pad to block boundary
            padding = RECORDSIZE - (info.size % RECORDSIZE)
            if padding != RECORDSIZE:
                self._handle.write(b"\0" * padding)

    def _write_header(self, info):
        """Write a tar header for the given TarInfo."""
        header = bytearray(RECORDSIZE)
        name_bytes = info.name.encode("utf-8", errors="replace")[:NAMSIZE]
        header[:len(name_bytes)] = name_bytes
        # Mode (octal)
        mode_str = f"{info.mode:06o}".encode()
        header[100:100+len(mode_str)] = mode_str
        header[108] = b"0"[0]
        header[109] = b"0"[0]
        # Size (octal)
        size_str = f"{info.size:011o}".encode()
        header[124:124+len(size_str)] = size_str
        # Mtime (octal)
        mtime_str = f"{info.mtime:011o}".encode()
        header[136:136+len(mtime_str)] = mtime_str
        # Type flag
        if info.type == DIRTYPE:
            header[156] = ord("5")
        else:
            header[156] = ord("0")
        # Magic + version
        header[257:263] = b"ustar\0"
        header[263:265] = b"00"
        # Owner name
        uname_bytes = info.uname.encode()[:32]
        header[265:265+len(uname_bytes)] = uname_bytes
        # Group name
        gname_bytes = info.gname.encode()[:32]
        header[297:297+len(gname_bytes)] = gname_bytes
        # Calculate checksum
        chksum = sum(header[:])
        chksum_str = f"{chksum:06o}\0 ".encode()
        header[148:148+len(chksum_str)] = chksum_str
        self._handle.write(bytes(header))

    def close(self):
        if self._closed:
            return
        if self.mode in ("w", "a"):
            # Write end-of-archive markers
            self._handle.write(b"\0" * RECORDSIZE * 2)
        if self.fileobj is None:
            self._handle.close()
        self._closed = True

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()


# ── Module-level convenience functions ──────────────────────────────────────

def open(name, mode="r", fileobj=None, **kwargs):
    """Open a tar archive."""
    return TarFile(name, mode, fileobj=fileobj, **kwargs)


def is_tarfile(name):
    """Check if a file is a tar archive (by magic bytes)."""
    try:
        with open(name, "rb") as f:
            magic = f.read(8)
            # Check for ustar magic
            return magic[257:263] == b"ustar" or magic[:3] == b"ust"
    except Exception:
        return False
