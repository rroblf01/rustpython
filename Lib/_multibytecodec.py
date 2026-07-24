"""Multibyte codec module stub."""


def __getattr__(name):
    """Return a function for any codec requested."""
    import codecs
    def codec_func(*args, **kwargs):
        return (b'', 0)
    return codec_func
