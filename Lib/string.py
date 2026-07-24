"""String template library stub."""

import re


class Template:
    """String template with $variable substitution."""

    delimiter = '$'
    idpattern = r'[_a-z][_a-z0-9]*'
    flags = re.IGNORECASE

    def __init__(self, template):
        self.template = template

    def safe_substitute(self, mapping={}, **kwargs):
        result = self.template
        d = dict(mapping)
        d.update(kwargs)
        for key, value in d.items():
            result = result.replace(f'${{{key}}}', str(value))
            result = result.replace(f'${key}', str(value))
        return result

    def substitute(self, mapping={}, **kwargs):
        result = self.safe_substitute(mapping, **kwargs)
        return result
