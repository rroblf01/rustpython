import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

print("Step 1: importing django.conf")
from django.conf import settings
print("Step 2: settings imported, configuring")
settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=['django.contrib.contenttypes', 'django.contrib.auth'])
print("Step 3: importing django")
import django
print("Step 4: django.setup()")
django.setup()
print("DONE - django.setup() succeeded")
