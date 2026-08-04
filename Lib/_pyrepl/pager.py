"""Minimal `_pyrepl.pager` for pydoc.

Real implementation paginates to $PAGER / less / more. Ours falls back to
printing the text (callers capture the output via io.StringIO, which is
all test_enum's test_pydoc needs).
"""

import os
import sys


def plain(text):
    return text


def get_pager():
    return plain_pager


def plain_pager(text):
    sys.stdout.write(text)


def pipe_pager(text, cmd):
    if sys.platform == "win32":
        os.system("more")
    else:
        try:
            import subprocess
            subprocess.run(cmd, shell=True, input=text.encode("utf-8"))
        except Exception:
            plain_pager(text)


def tempfile_pager(text):
    plain_pager(text)


def tty_pager(text):
    plain_pager(text)
