"""XML SAX handler module."""


class ContentHandler:
    def __init__(self):
        pass

    def startDocument(self):
        pass

    def endDocument(self):
        pass

    def startElement(self, name, attrs):
        pass

    def endElement(self, name):
        pass

    def characters(self, content):
        pass

    def processingInstruction(self, target, data):
        pass

    def setDocumentLocator(self, locator):
        pass

    def skippedEntity(self, name):
        pass

    def startPrefixMapping(self, prefix, uri):
        pass

    def endPrefixMapping(self, prefix):
        pass

    def ignorableWhitespace(self, whitespace):
        pass


class ErrorHandler:
    def __init__(self):
        pass

    def error(self, exception):
        raise exception

    def fatalError(self, exception):
        raise exception

    def warning(self, exception):
        pass

class DTDHandler:
    pass

class EntityResolver:
    pass

feature_namespaces = 'http://xml.org/sax/features/namespaces'
feature_namespace_prefixes = 'http://xml.org/sax/features/namespace-prefixes'
