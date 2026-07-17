import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

print("P1: importing settings")
from django.conf import settings
print("P2: configuring")
settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=['django.contrib.contenttypes', 'django.contrib.auth'])
print("P3: importing django")
import django
print("P4: django.setup()")
django.setup()
print("P5: DONE")
