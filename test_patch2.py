import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

from django.conf import UserSettingsHolder

# Patch __setattr__ to avoid triggering __getattr__ for _deleted
# Use super() via class hierarchy to avoid stack underflow
def patched_setattr(self, name, value):
    # Direct __dict__ access instead of self._deleted
    self.__dict__.get('_deleted', set()).discard(name)
    self.__dict__[name] = value
UserSettingsHolder.__setattr__ = patched_setattr

print("Patched")

from django.conf import settings
print("got settings")

try:
    settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=['django.contrib.contenttypes', 'django.contrib.auth'])
    print("configure() OK")
except Exception as e:
    print("FAIL:", type(e).__name__, str(e))
    import traceback
    traceback.print_exc()
