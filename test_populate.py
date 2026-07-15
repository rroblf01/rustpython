import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

from django.conf import UserSettingsHolder
def _patched_setattr(self, name, value):
    self.__dict__.get('_deleted', set()).discard(name)
    self.__dict__[name] = value
UserSettingsHolder.__setattr__ = _patched_setattr

# Test 1: Empty INSTALLED_APPS
print("=== Test 1: django.setup() with no INSTALLED_APPS ===")
from django.conf import settings
settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=[])
import django
try:
    django.setup()
    print("OK: setup() with empty INSTALLED_APPS")
except Exception as e:
    print("FAIL:", type(e).__name__, str(e))

print()
print("=== Test 2: import apps module directly ===")
try:
    from django.apps import apps
    print("OK: apps imported", type(apps))
except Exception as e:
    print("FAIL:", type(e).__name__, str(e))

print()
print("=== Test 3: populate with a simple app ===")
try:
    apps.populate(['django.contrib.contenttypes'])
    print("OK: populated contenttypes")
except Exception as e:
    print("FAIL:", type(e).__name__, str(e))
    import traceback
    traceback.print_exc()

print("done")
