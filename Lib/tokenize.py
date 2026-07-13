"""Tokenization for Python source code (RustPython-compatible simplified version).

Provides tokenize, generate_tokens, TokenInfo, untokenize, detect_encoding.
This is a simplified but functional tokenizer that works within RustPython's
regex and parsing capabilities.
"""

import token as _token_mod
from token import (
    ENDMARKER, NAME, NUMBER, STRING, NEWLINE, INDENT, DEDENT,
    COMMENT, NL, ERRORTOKEN, ENCODING, OP, EXACT_TOKEN_TYPES,
)

__all__ = ['tokenize', 'generate_tokens', 'detect_encoding', 'untokenize',
           'TokenInfo', 'TokenError', 'COMMENT', 'NL', 'ENCODING',
           'ENDMARKER', 'NAME', 'NUMBER', 'STRING', 'NEWLINE', 'INDENT',
           'DEDENT', 'OP', 'ERRORTOKEN']


class TokenError(Exception):
    """Raised when a token cannot be processed."""
    pass


class TokenInfo:
    """TokenInfo(type, string, start, end, line) - Named tuple for token info."""
    
    __slots__ = ('type', 'string', 'start', 'end', 'line')
    
    def __init__(self, type, string, start, end, line):
        self.type = type
        self.string = string
        self.start = start
        self.end = end
        self.line = line
    
    def __iter__(self):
        return iter([self.type, self.string, self.start, self.end, self.line])
    
    def __len__(self):
        return 5
    
    def __getitem__(self, index):
        items = [self.type, self.string, self.start, self.end, self.line]
        return items[index]
    
    def __repr__(self):
        return 'TokenInfo(type=' + repr(self.type) + ', string=' + repr(self.string) + ', start=' + repr(self.start) + ', end=' + repr(self.end) + ', line=' + repr(self.line) + ')'


# Python keyword list
_keywords = frozenset([
    'False', 'None', 'True', 'and', 'as', 'assert', 'async', 'await',
    'break', 'class', 'continue', 'def', 'del', 'elif', 'else', 'except',
    'finally', 'for', 'from', 'global', 'if', 'import', 'in', 'is',
    'lambda', 'nonlocal', 'not', 'or', 'pass', 'raise', 'return',
    'try', 'while', 'with', 'yield',
])

# Operators/delimiters sorted by length (longest first)
_ops = [
    '**=', '<<=', '>>=', '//=', '+=', '-=', '*=', '/=', '%=', '&=',
    '|=', '^=', '@=', '==', '!=', '<=', '>=', '<<', '>>', '**', '//',
    '->', ':=', '...',
]
_ops_set = frozenset(_ops)

_single_ops = '()[]{}:.,;+-*/%&|^~<>=@!'

# Whitespace characters
_whitespace = ' \t\f'


def _classify_name(name):
    """Classify a name token."""
    if name in _keywords:
        return NAME  # No special keyword token in our simplified version
    return NAME


def _tokenize_line(line, start_line=1):
    """Tokenize a single line and yield TokenInfo objects."""
    lineno = start_line
    line_orig = line
    line = line_orig.rstrip('\r\n')
    pos = 0
    length = len(line)
    
    while pos < length:
        ch = line[pos]
        
        # Whitespace
        if ch in _whitespace:
            start = pos
            while pos < length and line[pos] in _whitespace:
                pos += 1
            yield TokenInfo(NL, line[start:pos], (lineno, start), (lineno, pos), line_orig)
            continue
        
        # Comment
        if ch == '#':
            comment = line[pos:]
            yield TokenInfo(COMMENT, comment, (lineno, pos), (lineno, length), line_orig)
            pos = length
            continue
        
        # Strings (simplified)
        if ch in ("'", '"') or (ch in 'bBrRuufF' and pos + 1 < length and line[pos + 1] in ("'", '"')):
            # Check for raw/b/f string prefix
            prefix = ''
            string_start = pos
            while pos < length and line[pos] in 'bBrRuufF':
                prefix += line[pos]
                pos += 1
            quote = line[pos]
            pos += 1
            
            triple = (pos + 1 < length and line[pos] == quote and line[pos + 1] == quote)
            if triple:
                pos += 2  # Skip two more quote chars
            
            # Find end of string
            while pos < length:
                if triple and line[pos:pos+3] == quote * 3:
                    pos += 3
                    break
                elif not triple and line[pos] == quote:
                    pos += 1
                    break
                elif line[pos] == '\\' and pos + 1 < length:
                    pos += 2  # Skip escaped char
                else:
                    pos += 1
            else:
                # Unterminated string - just consume rest of line
                pos = length
            
            yield TokenInfo(STRING, line[string_start:pos], (lineno, string_start), (lineno, pos), line_orig)
            continue
        
        # Number (simplified)
        if ch.isdigit() or (ch == '.' and pos + 1 < length and line[pos + 1].isdigit()):
            start = pos
            # Check for 0x, 0o, 0b prefixes
            if ch == '0' and pos + 1 < length and line[pos + 1] in 'xXoObB':
                pos += 2
                while pos < length and (line[pos].isalnum() or line[pos] in '_'):
                    pos += 1
            else:
                while pos < length and (line[pos].isdigit() or line[pos] in '._eEjJxXoObBa-fA-F'):
                    if line[pos] in 'eE':
                        pos += 1
                        if pos < length and line[pos] in '+-':
                            pos += 1
                        continue
                    if line[pos] in 'xXoObB' and pos > start + 1:
                        pass  # Already handled
                    pos += 1
                    # Stop if we hit non-number chars
                    if pos < length and not (line[pos].isdigit() or line[pos] in '._eEjJxXoObBa-fA-F'):
                        break
            
            yield TokenInfo(NUMBER, line[start:pos], (lineno, start), (lineno, pos), line_orig)
            continue
        
        # Name/identifier
        if ch.isalpha() or ch == '_':
            start = pos
            while pos < length and (line[pos].isalnum() or line[pos] == '_'):
                pos += 1
            name = line[start:pos]
            tok_type = _classify_name(name)
            yield TokenInfo(tok_type, name, (lineno, start), (lineno, pos), line_orig)
            continue
        
        # Multi-char operators
        op_found = False
        for op in _ops:
            if line[pos:pos+len(op)] == op:
                yield TokenInfo(OP, op, (lineno, pos), (lineno, pos + len(op)), line_orig)
                pos += len(op)
                op_found = True
                break
        
        if op_found:
            continue
        
        # Single-char operators
        if ch in _single_ops:
            yield TokenInfo(OP, ch, (lineno, pos), (lineno, pos + 1), line_orig)
            pos += 1
            continue
        
        # Unknown character
        yield TokenInfo(ERRORTOKEN, ch, (lineno, pos), (lineno, pos + 1), line_orig)
        pos += 1


def generate_tokens(readline):
    """Tokenize a source reading readline calls.
    
    Yields TokenInfo objects. This is a simplified tokenizer; it handles
    the common cases needed for import and basic source analysis.
    """
    lineno = 1
    tokens = []
    
    while True:
        line = readline()
        if not line:
            break
        
        # Include the newline character as a NEWLINE token
        has_newline = line.endswith('\n')
        
        line_tokens = list(_tokenize_line(line, lineno))
        tokens.extend(line_tokens)
        
        if has_newline:
            newline_pos = len(line)
            if line.endswith('\r\n'):
                newline_str = '\r\n'
                newline_len = 2
            elif line.endswith('\r'):
                newline_str = '\r'
                newline_len = 1
            else:
                newline_str = '\n'
                newline_len = 1
            tokens.append(TokenInfo(NEWLINE, newline_str, (lineno, newline_pos - newline_len + 1), (lineno, newline_pos + 1), line))
        
        lineno += 1
    
    # Add end marker
    if tokens:
        last_line = tokens[-1].line
        tokens.append(TokenInfo(ENDMARKER, '', (lineno, 0), (lineno, 0), last_line))
    else:
        tokens.append(TokenInfo(ENDMARKER, '', (1, 0), (1, 0), ''))
    
    for tok in tokens:
        yield tok


def tokenize(readline):
    """Same as generate_tokens but yields (toknum, tokval) pairs."""
    for tok in generate_tokens(readline):
        yield (tok.type, tok.string)


def detect_encoding(readline):
    """Detect encoding from first two lines. Returns (encoding, lines_read)."""
    lines_read = []
    try:
        first = readline()
        if not first:
            return 'utf-8', []
        lines_read.append(first)
        
        # Check for encoding cookie: coding:xxx or coding=xxx
        import re as _re
        enc_pat = _re.compile(r'^[ \t\f]*#.*?coding[:=][ \t]*([-\w.]+)')
        m = enc_pat.match(first)
        if m:
            return m.group(1), lines_read
        
        second = readline()
        if not second:
            return 'utf-8', lines_read
        lines_read.append(second)
        m = enc_pat.match(second)
        if m:
            return m.group(1), lines_read
        
        return 'utf-8', lines_read
    except Exception:
        return 'utf-8', lines_read


def untokenize(iterable):
    """Untokenize tokens back into a string."""
    result = []
    prev_end = (0, 0)
    
    for tok in iterable:
        if isinstance(tok, TokenInfo):
            tok_type = tok.type
            tok_string = tok.string
            start = tok.start
            end = tok.end
        elif isinstance(tok, tuple) and len(tok) == 2:
            tok_type, tok_string = tok
            start = (0, 0)
            end = (0, 0)
        else:
            tok_type, tok_string = tok[0], tok[1]
            start = tok[2] if len(tok) > 2 else (0, 0)
            end = tok[3] if len(tok) > 3 else (0, 0)
        
        if tok_type in (ENCODING,):
            continue
        
        if tok_type == NEWLINE:
            result.append(tok_string)
            prev_end = end
        elif tok_type == NL:
            continue
        elif tok_type == COMMENT:
            result.append(tok_string)
        elif tok_type == ENDMARKER:
            break
        else:
            # Add spacing between tokens
            if prev_end[0] < start[0]:
                result.append('\n')
                for _ in range(start[1]):
                    result.append(' ')
            elif start[1] > prev_end[1] and prev_end[0] == start[0]:
                result.append(' ' * (start[1] - prev_end[1]))
            result.append(tok_string)
            prev_end = end
    
    return ''.join(result)


def open(filename):
    """Open a file for reading with encoding detection."""
    import io
    with io.open(filename, 'rb') as f:
        encoding, _ = detect_encoding(f.readline)
        f.seek(0)
        return io.open(filename, encoding=encoding)
