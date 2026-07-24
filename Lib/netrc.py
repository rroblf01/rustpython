"""netrc module stub."""


class netrc:
    def __init__(self, file=None):
        self.hosts = {}
        self.macros = {}

    def authenticators(self, host):
        return None

    def __repr__(self):
        return f'<netrc object>'
