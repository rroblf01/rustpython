"""Logging configuration module."""
from logging import Logger, Handler, getLogger
import sys


def fileConfig(fname, defaults=None, disable_existing_loggers=True):
    """Read logging configuration from a config file."""
    pass


def dictConfig(config):
    """Read logging configuration from a dictionary."""
    pass


class ConvertingMixin:
    """Mixin for converting values during configuration."""
    pass


class ConvertingDict(dict, ConvertingMixin):
    pass


class ConvertingList(list, ConvertingMixin):
    pass


class ConvertingTuple(tuple, ConvertingMixin):
    pass
