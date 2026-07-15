import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

from django.conf import UserSettingsHolder
def _patched_setattr(self, name, value):
    self.__dict__.get('_deleted', set()).discard(name)
    self.__dict__[name] = value
UserSettingsHolder.__setattr__ = _patched_setattr

print("1")
import django
print("2")

# Setup but empty INSTALLED_APPS
from django.conf import settings
print("3")
settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=[])
print("4")
from django.apps import apps
print("5: apps =", type(apps))
print("6: calling apps.populate")
apps.populate(settings.INSTALLED_APPS)
print("7: populated ok")
print("DONE")
