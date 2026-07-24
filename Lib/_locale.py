"""Minimal _locale module stub."""

class Error(Exception):
    pass


# LC_* constants
LC_ALL = 6
LC_COLLATE = 3
LC_CTYPE = 0
LC_MONETARY = 4
LC_NUMERIC = 1
LC_TIME = 2
LC_MESSAGES = 5


def setlocale(category, locale=None):
    """Set/get locale."""
    import os
    if locale is not None:
        os.environ['LANG'] = str(locale)
    return 'C'


def getlocale(category=LC_ALL):
    """Get locale as (language, encoding)."""
    return ('C', 'UTF-8')


def getdefaultlocale():
    """Get default locale."""
    return ('en_US', 'UTF-8')


def getpreferredencoding():
    """Get preferred encoding."""
    return 'UTF-8'


def localeconv():
    """Get locale conventions."""
    return {
        'decimal_point': '.',
        'thousands_sep': '',
        'grouping': [],
        'currency_symbol': '',
        'mon_decimal_point': '.',
        'mon_thousands_sep': '',
        'mon_grouping': [],
        'positive_sign': '',
        'negative_sign': '-',
        'int_frac_digits': 2,
        'frac_digits': 2,
        'p_cs_precedes': 1,
        'n_cs_precedes': 1,
        'p_sep_by_space': 0,
        'n_sep_by_space': 0,
        'p_sign_posn': 1,
        'n_sign_posn': 1,
        'int_curr_symbol': '',
    }


def strcoll(a, b):
    """Compare strings."""
    return (a > b) - (a < b)


def strxfrm(s):
    """Transform string for locale comparison."""
    return s


# These are OS-specific locale encoding functions
def get_encoding(category=LC_CTYPE):
    """Get encoding for a locale category."""
    return 'UTF-8'
