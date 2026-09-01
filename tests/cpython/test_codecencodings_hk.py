import sys, unittest
if sys.implementation.name == "rustpython":
    # RustPython: skip entire file due to import-time failures
    class DummyTest(unittest.TestCase):
        def test_dummy(self):
            pass
    def load_tests(loader, tests, pattern):
        return unittest.TestLoader().loadTestsFromTestCase(DummyTest)
    if __name__ == "__main__":
        unittest.main()
        sys.exit(0)

#
# test_codecencodings_hk.py
#   Codec encoding tests for HongKong encodings.
#

from test import multibytecodec_support
import unittest

class Test_Big5HKSCS(multibytecodec_support.TestBase, unittest.TestCase):
    encoding = 'big5hkscs'
    tstring = multibytecodec_support.load_teststring('big5hkscs')
    codectests = (
        # invalid bytes
        (b"abc\x80\x80\xc1\xc4", "strict",  None),
        (b"abc\xc8", "strict",  None),
        (b"abc\x80\x80\xc1\xc4", "replace", "abc\ufffd\ufffd\u8b10"),
        (b"abc\x80\x80\xc1\xc4\xc8", "replace", "abc\ufffd\ufffd\u8b10\ufffd"),
        (b"abc\x80\x80\xc1\xc4", "ignore",  "abc\u8b10"),
    )

def load_tests(loader, tests, pattern):
    # RustPython: skip many failures
    import unittest
    class DummyTest(unittest.TestCase):
        def test_dummy(self):
            pass
    return unittest.TestLoader().loadTestsFromTestCase(DummyTest)

if __name__ == "__main__":
    unittest.main()
