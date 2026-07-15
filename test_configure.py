import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

from django.conf import settings
print("got settings", type(settings))
try:
    settings.configure(DEBUG=True)
    print("configured")
except Exception as e:
    print("exception:", type(e).__name__, str(e))
    import traceback
    traceback.print_exc()

print("done")
