import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

import traceback

print("Step 1: importing django.conf")
try:
    from django.conf import settings
    print("Step 2: settings imported")
except Exception as e:
    print("FAIL at step 2:", type(e).__name__, str(e))
    traceback.print_exc()
    sys.exit(1)

print("Step 3: settings.configure()")
try:
    settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=['django.contrib.contenttypes', 'django.contrib.auth'])
    print("Step 4: configured OK")
except Exception as e:
    print("FAIL at step 4:", type(e).__name__, str(e))
    traceback.print_exc()
    sys.exit(1)

print("Step 5: import django")
try:
    import django
    print("Step 6: django imported, version =", django.VERSION)
except Exception as e:
    print("FAIL at step 6:", type(e).__name__, str(e))
    traceback.print_exc()
    sys.exit(1)

print("Step 7: django.setup()")
try:
    django.setup()
    print("Step 8: setup OK")
except Exception as e:
    print("FAIL at step 8:", type(e).__name__, str(e))
    traceback.print_exc()
    sys.exit(1)

print("ALL DONE - Django setup successful")
