# Async/Await Implementation Plan

## Changes needed:

### 1. compiler.rs - Fix Await expression compilation
- Change from just `GET_AWAITABLE` to full loop with SEND/YIELD_VALUE/END_SEND

### 2. vm.rs - Fix SEND opcode
- On StopIteration, jump to `arg` target instead of pushing None
- Also support Instance objects with `send` method (for FutureAwaitIterator)

### 3. object.rs - Add FutureAwaitIterator variant to PyObject
- New type that implements the await protocol for Futures
- Stores: future reference, yielded flag

### 4. misc.rs - Rewrite asyncio module
- EventLoop class with call_soon, call_later, run_until_complete
- Fix sleep() with proper delay mechanism
- Fix run() to use event loop
- Add ensure_future()
- Fix Future.__await__ to return FutureAwaitIterator
- Fix Task to use event loop for driving coroutines
