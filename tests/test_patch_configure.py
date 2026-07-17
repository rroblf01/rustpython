import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

# First, import the module to patch
from django.conf import UserSettingsHolder

# Patch __setattr__ to use object.__getattribute__ instead of self._deleted
# This works around RustPython bug where __getattr__ is called even for __dict__ entries
original_setattr = UserSettingsHolder.__setattr__
def patched_setattr(self, name, value):
    object.__getattribute__(self, '_deleted').discard(name)
    object.__setattr__(self, name, value)
UserSettingsHolder.__setattr__ = patched_setattr

print("Patched UserSettingsHolder.__setattr__")

from django.conf import settings
print("got settings:", type(settings))

try:
    settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=['django.contrib.contenttypes', 'django.contrib.auth'])
    print("settings.configure() OK")
except Exception as e:
    print("FAIL:", type(e).__name__, str(e))
    import traceback
    traceback.print_exc()

print("Phase 1 complete")
