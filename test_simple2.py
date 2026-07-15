import sys
print("1:", sys.path[:5])
from django.conf import settings
print("2: got settings")
print("3: settings module:", type(settings))
settings.configure(DEBUG=True)
print("4: configured")
import django
print("5: imported django")
print("6: version:", django.VERSION)
