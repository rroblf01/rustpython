"""Minimal RustPython stub for asyncio.base_events.

Real CPython's ``asyncio.base_events`` implements the actual
``BaseEventLoop`` that every concrete event loop (selector-based, proactor,
etc.) subclasses. This project's ``asyncio`` has no real event loop (see
``Lib/asyncio/events.py`` — ``get_running_loop()`` always raises), so this
stub exists only so that ``from asyncio import base_events`` resolves (and
``base_events.__file__`` is available, e.g. for stack-trace-shaped test
fixtures that reference it) — it re-exports the stub ``BaseEventLoop``
already defined in ``events.py`` rather than duplicating it.
"""

from asyncio.events import BaseEventLoop

__all__ = ('BaseEventLoop',)
