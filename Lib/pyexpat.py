"""pyexpat module stub."""

class ExpatError(Exception):
    pass


class XMLParserType:
    def __init__(self):
        self.buffer_text = False
        self.returns_unicode = True

    def StartElementHandler(self, handler):
        pass

    def EndElementHandler(self, handler):
        pass

    def CharacterDataHandler(self, handler):
        pass

    def StartNamespaceDeclHandler(self, handler):
        pass

    def EndNamespaceDeclHandler(self, handler):
        pass

    def ProcessingInstructionHandler(self, handler):
        pass

    def CommentHandler(self, handler):
        pass

    def StartCdataSectionHandler(self, handler):
        pass

    def EndCdataSectionHandler(self, handler):
        pass

    def DefaultHandler(self, handler):
        pass

    def DefaultHandlerExpand(self, handler):
        pass

    def ExternalEntityRefHandler(self, handler):
        pass

    def OrderedAttributes(self):
        return False

    def SetParamEntityParsing(self, flag):
        pass

    def Parse(self, data, isfinal=False):
        pass

    def GetErrorCode(self):
        return None

    def ErrorString(self, code):
        return ''

    def Create(self, encoding=None, namespace_separator=None):
        return XMLParserType()


XMLParserType = XMLParserType()
ParserCreate = XMLParserType.Create


errors = {
    1: 'out of memory',
    2: 'syntax error',
    3: 'no element found',
    4: 'not well-formed',
    5: 'unclosed token',
    6: 'unclosed CDATA section',
}

native_encoding = 'UTF-8'

# `expat.version_info`/`EXPAT_VERSION` — this is a stub, not a real Expat
# binding, so there's no genuine underlying library version to report; a
# plausible, recent-looking tuple is enough for code that just gates a
# feature check on it rather than asserting an exact value.
version_info = (2, 6, 0)
EXPAT_VERSION = 'expat_2.6.0'
