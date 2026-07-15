# asyncio.tasks stub for RustPython
# Minimal task-related stubs for Django and asgiref.

from asyncio.futures import Future, Task, ensure_future

def sleep(delay, result=None):
    return Future()

def gather(*coros_or_futures, return_exceptions=False):
    results = []
    for c in coros_or_futures:
        try:
            results.append(c)
        except Exception as e:
            if return_exceptions:
                results.append(e)
            else:
                raise
    f = Future()
    f.set_result(results)
    return f

def wait(futures, *, return_when='ALL_COMPLETED'):
    done = set()
    pending = set()
    for f in futures:
        if hasattr(f, 'done') and f.done():
            done.add(f)
        else:
            pending.add(f)
    return (done, pending)
