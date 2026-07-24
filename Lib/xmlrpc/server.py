"""XML-RPC server stub."""

class SimpleXMLRPCServer:
    def __init__(self, addr, requestHandler=None, logRequests=True, allow_none=False, encoding=None, bind_and_activate=True):
        self.addr = addr
        self.server = None

    def register_function(self, function, name=None):
        pass

    def register_introspection_functions(self):
        pass

    def register_instance(self, instance, allow_dotted_names=False):
        pass

    def serve_forever(self):
        pass

    def shutdown(self):
        pass


class DocXMLRPCServer(SimpleXMLRPCServer):
    pass


class SimpleXMLRPCRequestHandler:
    pass


class CGIXMLRPCRequestHandler:
    def __init__(self):
        pass

    def handle_request(self, request_text=''):
        pass

    def register_function(self, func, name=None):
        pass

    def register_instance(self, inst):
        pass

    def register_introspection_functions(self):
        pass


class ServerProxy:
    def __init__(self, uri, transport=None, encoding=None, verbose=False, allow_none=False, use_datetime=False, use_builtin_types=False, *, headers=(), context=None):
        self.__uri = uri
