"""FTP client module."""

import socket


class FTP:
    """FTP client class."""

    def __init__(self, host='', user='', passwd='', acct='', timeout=None):
        self.sock = None
        if host:
            self.connect(host)
            if user:
                self.login(user, passwd, acct)

    def connect(self, host='', port=0, timeout=None):
        if not port:
            port = 21
        self.sock = socket.create_connection((host, port), timeout)
        self.file = self.sock.makefile('r')
        self._getresp()
        return self.sock

    def _getresp(self):
        """Get response."""
        line = self.file.readline()
        return line.strip()

    def sendcmd(self, cmd):
        """Send a command."""
        self.sock.sendall(f'{cmd}\r\n'.encode())
        return self._getresp()

    def login(self, user='anonymous', passwd='', acct=''):
        """Login."""
        self.sendcmd(f'USER {user}')
        if passwd:
            self.sendcmd(f'PASS {passwd}')
        if acct:
            self.sendcmd(f'ACCT {acct}')
        return '230 Login successful'

    def retrbinary(self, cmd, callback, blocksize=8192, rest=None):
        """Retrieve data in binary mode."""
        self.sendcmd('TYPE I')
        self.sendcmd(f'{cmd}')
        while True:
            data = self.sock.recv(blocksize)
            if not data:
                break
            callback(data)
        return self._getresp()

    def retrlines(self, cmd, callback=None):
        """Retrieve data in text mode."""
        self.sendcmd(f'{cmd}')
        data = self.file.read()
        if callback:
            for line in data.split('\n'):
                if line:
                    callback(line)
        return self._getresp()

    def storbinary(self, cmd, fp, blocksize=8192):
        """Store data in binary mode."""
        self.sendcmd('TYPE I')
        self.sendcmd(f'{cmd}')
        while True:
            data = fp.read(blocksize)
            if not data:
                break
            self.sock.sendall(data)
        return self._getresp()

    def quit(self):
        """Quit."""
        self.sendcmd('QUIT')
        self.close()

    def close(self):
        """Close connection."""
        if self.sock:
            self.sock.close()
