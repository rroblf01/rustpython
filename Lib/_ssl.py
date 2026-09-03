# Stub for _ssl module - raises SkipTest when imported in tests that require it
import unittest
raise unittest.SkipTest("_ssl not available in RustPython")
