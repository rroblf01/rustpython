"""Support for running coroutines in parallel with staggered start times.

Minimal RustPython port of CPython's ``asyncio.staggered``. This project's
``asyncio`` package has no real event loop (see ``Lib/asyncio/events.py`` —
``get_running_loop()`` always raises, there is no scheduler actually driving
tasks concurrently), so the real algorithm's delay/cancellation machinery
(``locks.Event``, ``exceptions.TimeoutError``, ``tasks.wait_for``, real
``Task`` cancellation) can't be reproduced faithfully — those submodules
don't even exist here (only ``events``/``futures``/``tasks``/``coroutines``
do). This keeps the same public name/signature/return-shape contract
(``(winner_result, winner_index, exceptions)``) so `from asyncio import
staggered` and `asyncio.staggered.staggered_race(...)` work structurally,
but runs the coroutines sequentially (first success wins) rather than
staggered-concurrently with a real delay.
"""

__all__ = ('staggered_race',)


async def staggered_race(coro_fns, delay, *, loop=None):
    """Run coroutines sequentially, returning the first to succeed.

    Real CPython's version starts ``coro_fns`` staggered ``delay`` seconds
    apart and races them concurrently. Without a real event loop to run
    them concurrently on, this runs each one in turn (still honoring the
    "only the first success counts, exceptions from losers are collected"
    contract) and returns as soon as one succeeds.
    """
    exceptions = []
    for coro_fn in coro_fns:
        try:
            result = await coro_fn()
        except (SystemExit, KeyboardInterrupt):
            raise
        except BaseException as e:
            exceptions.append(e)
            continue
        else:
            exceptions.append(None)
            return result, len(exceptions) - 1, exceptions
    return None, None, exceptions
