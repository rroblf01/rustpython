"""XML SAX module stub."""

from xml.sax.handler import ContentHandler
import sys


class SAXReaderNotAvailable(SAXException):
    pass


def make_parser(parser_list=[]):
    return _ExpatParser()


def parse(source, handler, **kwargs):
    parser = make_parser()
    parser.setContentHandler(handler)
    parser.parse(source)


def parseString(string, handler, **kwargs):
    parser = make_parser()
    parser.setContentHandler(handler)
    parser.feed(string)
    parser.close()


class _ExpatParser:
    def __init__(self):
        self._handler = None

    def setContentHandler(self, handler):
        self._handler = handler

    def parse(self, source):
        import pyexpat
        if hasattr(source, 'read'):
            data = source.read()
        else:
            with open(source) as f:
                data = f.read()
        self.feed(data)
        self.close()

    def feed(self, data):
        if self._handler and hasattr(self._handler, 'characters'):
            self._handler.startDocument()
            self._handler.characters(str(data))
            self._handler.endDocument()

    def close(self):
        pass

    def reset(self):
        pass


class InputSource:
    def __init__(self, system_id=None):
        self._system_id = system_id

    def setSystemId(self, system_id):
        self._system_id = system_id

    def getSystemId(self):
        return self._system_id
