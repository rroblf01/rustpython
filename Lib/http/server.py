"""HTTP server stub."""
import socket
import sys


class HTTPServer:
    def __init__(self, server_address, RequestHandlerClass):
        self.server_address = server_address
        self.RequestHandlerClass = RequestHandlerClass


class BaseHTTPRequestHandler:
    def __init__(self, request, client_address, server):
        self.request = request
        self.client_address = client_address
        self.server = server
        self.rfile = request.makefile('rb')
        self.wfile = request.makefile('wb')
        self.raw_requestline = self.rfile.readline()
        self.parse_request()

    def parse_request(self):
        line = str(self.raw_requestline, 'utf-8')
        parts = line.split()
        if len(parts) >= 2:
            self.command = parts[0]
            self.path = parts[1]

    def send_response(self, code, message=None):
        self.wfile.write(f'HTTP/1.0 {code}\r\n'.encode())

    def send_header(self, keyword, value):
        self.wfile.write(f'{keyword}: {value}\r\n'.encode())

    def end_headers(self):
        self.wfile.write(b'\r\n')

    def log_message(self, format, *args):
        pass


class SimpleHTTPRequestHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        pass


server = HTTPServer
HTTPServer = HTTPServer
