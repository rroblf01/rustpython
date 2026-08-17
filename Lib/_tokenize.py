"""Minimal _tokenize stub for RustPython.

This provides a basic TokenizerIter that can be used by tokenize.py
when the C extension is not available.
"""

class TokenizerIter:
    """Minimal tokenizer iterator stub."""
    def __init__(self, readline, encoding=None, extra_tokens=False):
        self.readline = readline
        self.encoding = encoding
        self.extra_tokens = extra_tokens
        self._buffer = []
    
    def __iter__(self):
        return self
    
    def __next__(self):
        raise StopIteration
