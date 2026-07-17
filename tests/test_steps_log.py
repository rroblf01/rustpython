import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

log = open("/tmp/rp_debug_log.txt", "w")

from django.conf import UserSettingsHolder
def _patched_setattr(self, name, value):
    self.__dict__.get('_deleted', set()).discard(name)
    self.__dict__[name] = value
UserSettingsHolder.__setattr__ = _patched_setattr

log.write("1: import django\n")
import django
log.write("2: django = " + str(django) + "\n")

log.write("3: from django.conf import settings\n")
from django.conf import settings
log.write("4: settings type = " + str(type(settings)) + "\n")

log.write("5: settings.configure()\n")
settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=[])
log.write("6: configured\n")

log.write("7: from django.apps import apps\n")
try:
    from django.apps import apps
    log.write("8: apps = " + str(type(apps)) + "\n")
except Exception as e:
    log.write("8: FAIL: " + str(e) + "\n")

log.write("9: check settings.INSTALLED_APPS\n")
try:
    val = settings.INSTALLED_APPS
    log.write("10: INSTALLED_APPS = " + str(val) + "\n")
except Exception as e:
    log.write("10: FAIL: " + str(e) + "\n")

log.write("11: apps.populate()\n")
try:
    apps.populate(settings.INSTALLED_APPS)
    log.write("12: populated ok\n")
except Exception as e:
    log.write("12: FAIL: " + str(e) + "\n")
    import traceback
    traceback.print_exc(file=log)

log.write("DONE\n")
log.close()
