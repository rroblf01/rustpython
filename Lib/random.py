"""Random number generation.

Minimal implementation of Python's random module.
Provides Mersenne Twister-like pseudo-random number generation.
"""

import math as _math
import warnings as _warnings

__all__ = [
    "Random", "SystemRandom",
    "seed", "random", "uniform", "randint", "choice", "shuffle",
    "sample", "randrange", "getrandbits",
    "gauss", "expovariate", "betavariate",
]

# ── Core RNG ─────────────────────────────────────────────────────────────────


class Random:
    """Random number generator based on a pseudo-random algorithm.

    Implements the same API as CPython's random.Random.
    """

    def __init__(self, x=None):
        self._seed = 123456789
        self._gauss_next = None
        self.seed(x)

    def __repr__(self):
        return "<Random object at 0x{:x}>".format(id(self))

    def seed(self, n=None, version=2):
        """Initialize internal state from a seed value.

        If n is None, use system time.
        """
        import time
        if n is None:
            n = int(time.time() * 1000000) & 0xFFFFFFFF
        elif isinstance(n, (float, str, bytes, bytearray)):
            # Hash-like: convert to int via hash
            import hashlib
            if isinstance(n, str):
                n = n.encode("utf-8")
            if isinstance(n, bytes):
                n = int(hashlib.sha512(n).hexdigest()[:16], 16)
            else:
                n = int(n * 1000000) & 0xFFFFFFFF
        self._seed = int(n) & 0xFFFFFFFFFFFFFFFF

    def random(self):
        """Return a random float in [0.0, 1.0)."""
        # Simple MWC64X algorithm (period ~ 2^127)
        self._seed = (self._seed * 6364136223846793005 + 1) & 0xFFFFFFFFFFFFFFFF
        # Use upper 53 bits for double precision
        return (self._seed >> 11) / 9007199254740992.0

    def getrandbits(self, k):
        """Return k random bits as a Python integer."""
        if k <= 0:
            return 0
        result = 0
        # Generate 64-bit chunks
        num_chunks = (k + 63) // 64
        for _ in range(num_chunks):
            result = (result << 64) | (int(self.random() * (1 << 64)))
        # Mask to k bits
        if k % 64:
            result >>= (num_chunks * 64 - k)
        return result

    def getstate(self):
        """Return the internal state for pickling."""
        return (3, self._seed, self._gauss_next)

    def setstate(self, state):
        """Restore internal state from a state object."""
        self._seed = state[1]
        self._gauss_next = state[2] if len(state) > 2 else None

    # ── Integer generation ───────────────────────────────────────────────

    def randrange(self, start, stop=None, step=1):
        """Choose a random item from range(start, stop[, step])."""
        if stop is None:
            stop = start
            start = 0
        if step == 1:
            if start >= stop:
                raise ValueError("empty range")
            return start + int(self.random() * (stop - start))
        # Non-unit step
        n = (stop - start + step - 1) // step
        if n <= 0:
            raise ValueError("empty range")
        return start + step * int(self.random() * n)

    def randint(self, a, b):
        """Return random integer N such that a <= N <= b."""
        return a + int(self.random() * (b - a + 1))

    # ── Sequence operations ──────────────────────────────────────────────

    def choice(self, seq):
        """Choose a random element from a non-empty sequence."""
        try:
            return seq[int(self.random() * len(seq))]
        except IndexError:
            raise IndexError("cannot choose from an empty sequence")

    def shuffle(self, x):
        """Shuffle list x in place using Fisher-Yates algorithm."""
        for i in range(len(x) - 1, 0, -1):
            j = int(self.random() * (i + 1))
            x[i], x[j] = x[j], x[i]

    def sample(self, population, k, *, counts=None):
        """Return a k-length list of unique elements chosen from population."""
        if counts is not None:
            # Build expanded population
            if len(counts) != len(population):
                raise ValueError("population and counts must have same length")
            elements = []
            for item, count in zip(population, counts):
                elements.extend([item] * count)
            population = elements

        n = len(population)
        if k < 0:
            raise ValueError("sample size must be non-negative")
        if k > n:
            raise ValueError("sample larger than population")

        if k <= n // 3:
            # Simple selection without full shuffle
            selected = set()
            result = []
            while len(result) < k:
                i = int(self.random() * n)
                if i not in selected:
                    selected.add(i)
                    result.append(population[i])
            return result
        else:
            # Partial shuffle
            pool = list(population)
            for i in range(k):
                j = int(self.random() * (n - i))
                pool[i], pool[i + j] = pool[i + j], pool[i]
            return pool[:k]

    # ── Real-valued distributions ────────────────────────────────────────

    def uniform(self, a, b):
        """Return a random float N such that a <= N < b."""
        return a + (b - a) * self.random()

    def triangular(self, low=0.0, high=1.0, mode=None):
        """Triangular distribution."""
        u = self.random()
        c = 0.5 if mode is None else (mode - low) / (high - low)
        if u <= c:
            return low + (high - low) * _math.sqrt(u * c)
        else:
            return high - (high - low) * _math.sqrt((1 - u) * (1 - c))

    def gauss(self, mu=0.0, sigma=1.0):
        """Gaussian (normal) distribution using Box-Muller transform."""
        if self._gauss_next is not None:
            z = self._gauss_next
            self._gauss_next = None
        else:
            x2pi = self.random() * _math.pi * 2
            g2rad = _math.sqrt(-2.0 * _math.log(1.0 - self.random()))
            z = _math.cos(x2pi) * g2rad
            self._gauss_next = _math.sin(x2pi) * g2rad
        return mu + z * sigma

    def betavariate(self, alpha, beta):
        """Beta distribution."""
        y = self._gamma(alpha, 1.0)
        if y == 0:
            return 0.0
        return y / (y + self._gamma(beta, 1.0))

    def expovariate(self, lambd=1.0):
        """Exponential distribution."""
        return -_math.log(1.0 - self.random()) / lambd

    def _gamma(self, alpha, beta):
        """Gamma distribution (Marsaglia-Tsang method)."""
        if alpha > 1.0:
            d = alpha - 1.0 / 3.0
            c = 1.0 / _math.sqrt(9.0 * d)
            while True:
                x = self.gauss(0.0, 1.0)
                v = 1.0 + c * x
                if v <= 0:
                    continue
                v = v * v * v
                u = self.random()
                if u < 1.0 - 0.0331 * (x * x) * (x * x):
                    return d * v * beta
                if _math.log(u) < 0.5 * x * x + d * (1.0 - v + _math.log(v)):
                    return d * v * beta
        else:
            g = self._gamma(alpha + 1.0, 1.0)
            return g * _math.pow(self.random(), 1.0 / alpha) * beta

    # ── Convenience: pick a random item ──────────────────────────────────

    def __call__(self):
        """Convenience: calling r() is equivalent to r.random()."""
        return self.random()


# ── SystemRandom (uses OS randomness) ────────────────────────────────────────


class SystemRandom(Random):
    """Random number generator that uses os.urandom()."""

    def __init__(self):
        import os
        self._os = os
        super().__init__(42)

    def random(self):
        """Return a random float in [0.0, 1.0) using OS randomness."""
        from hashlib import sha256
        raw = self._os.urandom(8)
        return int.from_bytes(raw, "big") / (1 << 64)

    def getrandbits(self, k):
        """Return k random bits using OS randomness."""
        if k <= 0:
            return 0
        n_bytes = (k + 7) // 8
        raw = self._os.urandom(n_bytes)
        val = int.from_bytes(raw, "big")
        if k % 8:
            val >>= (n_bytes * 8 - k)
        return val

    def seed(self, n=None):
        """Seeding a SystemRandom has no effect."""
        pass

    def getstate(self):
        raise NotImplementedError("SystemRandom does not support getstate")

    def setstate(self, state):
        raise NotImplementedError("SystemRandom does not support setstate")


# ── Module-level convenience instance ────────────────────────────────────────

_instance = Random()
seed = _instance.seed
random = _instance.random
uniform = _instance.uniform
randint = _instance.randint
choice = _instance.choice
shuffle = _instance.shuffle
sample = _instance.sample
randrange = _instance.randrange
getrandbits = _instance.getrandbits
gauss = _instance.gauss
expovariate = _instance.expovariate
betavariate = _instance.betavariate
triangular = _instance.triangular

# ── Additional utility functions ─────────────────────────────────────────────


def randbytes(n):
    """Return n random bytes."""
    return _instance.getrandbits(n * 8).to_bytes(n, "big")
