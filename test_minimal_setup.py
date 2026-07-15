import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

print("STEP 1: importing settings")
from django.conf import settings

print("STEP 2: configure()")
settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=[])

print("STEP 3: import django")
import django

print("STEP 4: django.setup() (empty apps)")
django.setup()
print("STEP 5: SUCCESS - django.setup() completed!")
