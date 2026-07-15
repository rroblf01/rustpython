import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

# Monkey-patch UserSettingsHolder to use direct __dict__ access
from django.conf import global_settings
from django.conf import UserSettingsHolder as OriginalUserSettingsHolder

class PatchedUserSettingsHolder(OriginalUserSettingsHolder):
    def __setattr__(self, name, value):
        # Use object.__getattribute__ instead of self._deleted
        # to avoid triggering __getattr__ in RustPython
        object.__getattribute__(self, '_deleted').discard(name)
        super().__setattr__(name, value)

# Now try configuring settings with the patched class
from django.conf import LazySettings

settings = LazySettings()
print("got LazySettings")

try:
    settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=['django.contrib.contenttypes', 'django.contrib.auth'])
    print("settings.configure() OK")
except Exception as e:
    print("FAIL:", type(e).__name__, str(e))
    import traceback
    traceback.print_exc()

print("done")
