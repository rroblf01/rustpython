import sys
sys.path.insert(0, "/usr/lib/python3.13")

# Phase 1: Django setup
print("=== PHASE 1: Django setup ===")
try:
    from django.conf import settings
    settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=['django.contrib.contenttypes', 'django.contrib.auth'])
    import django
    django.setup()
    print("OK: django.setup() succeeded")
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()
    sys.exit(1)

# Phase 2: Import models and connection
print("\n=== PHASE 2: Import models and connection ===")
try:
    from django.db import models
    print("OK: models imported, type =", type(models))
except Exception as e:
    print("FAIL: models import:", repr(e))

try:
    from django.db import connection
    print("OK: connection imported, type =", type(connection))
except Exception as e:
    print("FAIL: connection import:", repr(e))

# Phase 3: Define a model
print("\n=== PHASE 3: Define TestModel ===")
try:
    class TestModel(models.Model):
        name = models.CharField(max_length=100)
        class Meta:
            app_label = 'test'
    print("OK: TestModel defined")
    print("    db_table =", TestModel._meta.db_table)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

# Phase 4: Create schema
print("\n=== PHASE 4: Create schema ===")
try:
    from django.db import connection
    with connection.schema_editor() as schema_editor:
        schema_editor.create_model(TestModel)
    print("OK: schema created")
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

# Phase 5: Create object
print("\n=== PHASE 5: TestModel.objects.create() ===")
try:
    obj = TestModel.objects.create(name='hello')
    print("OK: created obj, id =", obj.id, "name =", obj.name)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

# Phase 6: Query
print("\n=== PHASE 6: TestModel.objects.count() ===")
try:
    count = TestModel.objects.count()
    print("OK: count =", count)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

# Phase 7: QuerySet filter
print("\n=== PHASE 7: TestModel.objects.filter() ===")
try:
    qs = TestModel.objects.filter(name='hello')
    print("OK: filter returned", qs)
    print("    count =", qs.count())
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

# Phase 8: Get
print("\n=== PHASE 8: TestModel.objects.get() ===")
try:
    obj = TestModel.objects.get(name='hello')
    print("OK: get returned id =", obj.id, "name =", obj.name)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print("\n=== ALL DONE ===")
