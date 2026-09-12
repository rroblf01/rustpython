"""Minimal RustPython stub for asyncio.taskgroups.

Real CPython's ``TaskGroup`` structures concurrent child tasks with real
cancellation/exception-group semantics tied to the event loop. This
project's ``asyncio`` has no real event loop (see ``Lib/asyncio/events.py``),
so this is a best-effort structural stub: tasks handed to ``create_task()``
are awaited eagerly (sequentially) rather than scheduled concurrently. Good
enough for simple synchronous-style ``async with TaskGroup() as tg: ...``
usage, and so ``from asyncio import taskgroups`` resolves at all.
"""

from asyncio.futures import Task

__all__ = ('TaskGroup',)


class TaskGroup:
    def __init__(self):
        self._tasks = []
        self._entered = False
        self._exiting = False

    def __repr__(self):
        return f"<TaskGroup tasks={len(self._tasks)}>"

    async def __aenter__(self):
        self._entered = True
        return self

    async def __aexit__(self, et, exc, tb):
        self._exiting = True
        return False

    def create_task(self, coro, *, name=None, context=None):
        if not self._entered or self._exiting:
            raise RuntimeError(
                "TaskGroup.create_task() called outside of an active TaskGroup"
            )
        task = Task(coro)
        self._tasks.append(task)
        return task
