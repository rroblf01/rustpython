import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

from django.conf import global_settings
from django.conf import UserSettingsHolder
print("imports ok")

# Test UserSettingsHolder directly
try:
    holder = UserSettingsHolder(global_settings)
    print("holder created ok")
    holder.DEBUG = True
    print("set DEBUG ok")
    print("DEBUG =", holder.DEBUG)
except Exception as e:
    print("exception:", type(e).__name__, str(e))
    import traceback
    traceback.print_exc()
