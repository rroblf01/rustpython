#!/bin/bash
# Quick RustPython test script
# Usage: ./test.sh                    # default test
#        ./test.sh -c "code"          # inline code
#        ./test.sh file.py            # file test
# Env vars:
#   BUILD_PROFILE=release|release-lite  (default: debug)
#   FEATURES=jit,sqlite3                (default: jit)
#   TIMEOUT=30                          (default: 10)
#   VENV=/path/to/venv                   (default: /tmp/rustpython-django-env)

set -e

DEFAULT_CODE='import sys, os
print("RustPython ok!")
'

PROFILE="${BUILD_PROFILE:-}"
FEATURES="${FEATURES:-jit}"
TOUT="${TIMEOUT:-10}"
VENV="${VENV:-/tmp/rustpython-django-env}"

# Wrap code with sys.path for Django when needed
if echo "$*" | grep -q 'django\|Django'; then
    WRAPPER="import sys; sys.path.insert(0, '$VENV/lib/python3.14/site-packages'); "
else
    WRAPPER=""
fi

if [ "$1" = "-c" ]; then
    CODE="${WRAPPER}$2"
elif [ -n "$1" ] && [ -f "$1" ]; then
    CODE=$(cat "$1")
else
    CODE="${WRAPPER}$DEFAULT_CODE"
fi

echo "=== Build (features=$FEATURES, profile=$PROFILE) ==="
CMD="cargo build --features \"$FEATURES\""
if [ -n "$PROFILE" ]; then
    CMD="$CMD --profile $PROFILE"
fi
if ! eval "$CMD" 2>&1 | tail -3; then
    echo "Build failed"
    exit 1
fi

# Find binary
BIN="./target/debug/rustpython"
if [ "$PROFILE" = "release" ]; then
    BIN="./target/release/rustpython"
elif [ "$PROFILE" = "release-lite" ]; then
    BIN="./target/release-lite/rustpython"
fi

echo "=== Run ==="
timeout "$TOUT" "$BIN" -c "$CODE" 2>/dev/null || echo "(exit: $?)"
