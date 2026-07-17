import sys
sys.path.insert(0, "/usr/lib/python3.13")
sys.path.insert(0, "/opt/data/home/.local/share/uv/tools/django/lib/python3.13/site-packages")

# === PATCH: Work around RustPython __getattr__ bug ===
from django.conf import UserSettingsHolder
def _patched_setattr(self, name, value):
    self.__dict__.get('_deleted', set()).discard(name)
    self.__dict__[name] = value
UserSettingsHolder.__setattr__ = _patched_setattr

print("=" * 60)
print("PHASE 1: Django setup")
print("=" * 60)

from django.conf import settings
settings.configure(DEBUG=True, DATABASES={'default': {'ENGINE': 'django.db.backends.sqlite3', 'NAME': ':memory:'}}, INSTALLED_APPS=['django.contrib.contenttypes', 'django.contrib.auth'])
import django
django.setup()
print("OK: django.setup() succeeded")

print()
print("=" * 60)
print("PHASE 2: Import models and connection")
print("=" * 60)

try:
    from django.db import models
    print("OK: models imported, type =", type(models))
except Exception as e:
    print("FAIL: models import:", repr(e))
    import traceback
    traceback.print_exc()

try:
    from django.db import connection
    print("OK: connection imported, type =", type(connection))
except Exception as e:
    print("FAIL: connection import:", repr(e))

print()
print("=" * 60)
print("PHASE 3: Define TestModel")
print("=" * 60)

class TestModel(models.Model):
    name = models.CharField(max_length=100)
    class Meta:
        app_label = 'test'

print("OK: TestModel defined")
print("    db_table =", TestModel._meta.db_table)

print()
print("=" * 60)
print("PHASE 4: Create schema")
print("=" * 60)

try:
    with connection.schema_editor() as schema_editor:
        schema_editor.create_model(TestModel)
    print("OK: schema created")
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 5: TestModel.objects.create()")
print("=" * 60)

try:
    obj = TestModel.objects.create(name='hello')
    print("OK: created obj, id =", obj.id, "name =", obj.name)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 6: TestModel.objects.all()")
print("=" * 60)

try:
    all_objs = list(TestModel.objects.all())
    print("OK: all() returned", len(all_objs), "objects")
    for o in all_objs:
        print("   - id:", o.id, "name:", o.name)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 7: TestModel.objects.count()")
print("=" * 60)

try:
    count = TestModel.objects.count()
    print("OK: count =", count)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 8: TestModel.objects.filter()")
print("=" * 60)

try:
    qs = TestModel.objects.filter(name='hello')
    print("OK: filter returned queryset")
    print("    count =", qs.count())
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 9: TestModel.objects.get()")
print("=" * 60)

try:
    obj = TestModel.objects.get(name='hello')
    print("OK: get returned id =", obj.id, "name =", obj.name)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 10: Create second object and ordering")
print("=" * 60)

try:
    obj2 = TestModel.objects.create(name='world')
    print("OK: created second obj, id =", obj2.id, "name =", obj2.name)
    all_objs = list(TestModel.objects.all().order_by('name'))
    print("    ordered by name:")
    for o in all_objs:
        print("       - id:", o.id, "name:", o.name)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 11: Update object")
print("=" * 60)

try:
    updated = TestModel.objects.filter(name='hello').update(name='hello-updated')
    print("OK: updated", updated, "object(s)")
    obj = TestModel.objects.get(name='hello-updated')
    print("    verified: id =", obj.id, "name =", obj.name)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 12: Delete object")
print("=" * 60)

try:
    deleted = TestModel.objects.filter(name='hello-updated').delete()
    print("OK: deleted returned:", deleted)
    remaining = TestModel.objects.count()
    print("    remaining count:", remaining)
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("PHASE 13: Related objects (ForeignKey)")
print("=" * 60)

try:
    class Category(models.Model):
        name = models.CharField(max_length=50)
        class Meta:
            app_label = 'test'
    
    class Article(models.Model):
        title = models.CharField(max_length=200)
        category = models.ForeignKey(Category, on_delete=models.CASCADE)
        class Meta:
            app_label = 'test'
    
    print("OK: Related models defined")
    
    with connection.schema_editor() as schema_editor:
        schema_editor.create_model(Category)
        schema_editor.create_model(Article)
    print("OK: Related model schemas created")
    
    cat = Category.objects.create(name='Tech')
    art = Article.objects.create(title='RustPython', category=cat)
    print("OK: Created category id={}, article id={}".format(cat.id, art.id))
    print("    article.category_id =", art.category_id)
    print("    article.category =", art.category)
    print("    article.category.name =", art.category.name)
    
except Exception as e:
    print("FAIL:", repr(e))
    import traceback
    traceback.print_exc()

print()
print("=" * 60)
print("ALL TESTS COMPLETE")
print("=" * 60)
