"""Minimal _string module for string.Formatter support."""


def formatter_parser(format_string):
    """Parse a format string into fields."""
    # Simple implementation supporting basic field_name formats
    parts = []
    i = 0
    n = len(format_string)
    while i < n:
        # Find next {
        start = i
        while i < n and format_string[i] != '{' and format_string[i] != '}':
            i += 1
        if start < i:
            parts.append((format_string[start:i], None, None, None))
        if i >= n:
            break
        if format_string[i] == '}' and i + 1 < n and format_string[i + 1] == '}':
            parts.append(('}', None, None, None))
            i += 2
            continue
        if format_string[i] == '{' and i + 1 < n and format_string[i + 1] == '{':
            parts.append(('{', None, None, None))
            i += 2
            continue
        if format_string[i] == '}':
            parts.append(('}', None, None, None))
            i += 1
            continue
        # Parse field: {field_name:format_spec}
        i += 1  # skip {
        field_name = ''
        while i < n and format_string[i] != ':' and format_string[i] != '!' and format_string[i] != '}':
            field_name += format_string[i]
            i += 1
        conversion = None
        if i < n and format_string[i] == '!':
            i += 1
            if i < n:
                conversion = format_string[i]
                i += 1
        format_spec = None
        if i < n and format_string[i] == ':':
            i += 1
            depth = 1
            spec = ''
            while i < n and depth > 0:
                if format_string[i] == '{':
                    depth += 1
                elif format_string[i] == '}':
                    depth -= 1
                    if depth == 0:
                        break
                spec += format_string[i]
                i += 1
            if spec:
                format_spec = spec
        if i < n:
            i += 1  # skip }
        parts.append((None, field_name, format_spec, conversion))
    return parts


def formatter_field_name_split(field_name):
    """Split a field name into first part and an iterator over the rest."""
    if not field_name:
        return ('', iter([]))
    # Split at first . or [
    first = ''
    i = 0
    while i < len(field_name) and field_name[i] != '.' and field_name[i] != '[':
        first += field_name[i]
        i += 1
    rest = field_name[i:]
    def iter_rest():
        nonlocal rest
        while rest:
            if rest[0] == '.':
                rest = rest[1:]
                dot_name = ''
                while rest and rest[0] != '.' and rest[0] != '[':
                    dot_name += rest[0]
                    rest = rest[1:]
                yield (True, dot_name)  # True = attribute
            elif rest[0] == '[':
                rest = rest[1:]
                bracket = ''
                depth = 1
                while rest and depth > 0:
                    if rest[0] == '[':
                        depth += 1
                    elif rest[0] == ']':
                        depth -= 1
                        if depth == 0:
                            rest = rest[1:]
                            break
                    bracket += rest[0]
                    rest = rest[1:]
                # Check if it's an int index
                try:
                    yield (False, int(bracket))  # False = index
                except ValueError:
                    yield (False, bracket)
    return (first, iter_rest())


__all__ = ["formatter_parser", "formatter_field_name_split"]
