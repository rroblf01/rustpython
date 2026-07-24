"""POP3 client stub."""
import socket


class POP3:
    def __init__(self, host, port=110, timeout=None):
        self.host = host
        self.port = port
        self.sock = None
        if host:
            self.connect(host, port)

    def connect(self, host, port=110):
        self.sock = socket.create_connection((host, port))

    def getwelcome(self):
        return b'+OK POP3 server ready'

    def user(self, user):
        return b'+OK'

    def pass_(self, pswd):
        return b'+OK'

    def stat(self):
        return (0, 0)

    def list(self, which=None):
        return (b'+OK', [], 0)

    def retr(self, which):
        return (b'+OK', [b''], 0)

    def dele(self, which):
        return b'+OK'

    def quit(self):
        if self.sock:
            self.sock.close()
