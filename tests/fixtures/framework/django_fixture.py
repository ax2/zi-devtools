import os,sys,json,io
from pathlib import Path
root=Path(sys.argv[1]).resolve(); root.mkdir(parents=True,exist_ok=True)
if (root/'fixture.sqlite3').exists(): raise RuntimeError('Use a fresh fixture directory')
app=root/'fixture_shop'; (app/'migrations').mkdir(parents=True,exist_ok=True)
(app/'__init__.py').write_text(''); (app/'migrations/__init__.py').write_text('')
(app/'models.py').write_text('from django.db import models\nclass Item(models.Model):\n name=models.CharField(max_length=40)\n')
sys.path.insert(0,str(root))
from django.conf import settings
settings.configure(INSTALLED_APPS=['fixture_shop'],DATABASES={'default':{'ENGINE':'django.db.backends.sqlite3','NAME':str(root/'fixture.sqlite3')}},SECRET_KEY='fixture-only-not-a-credential',DEBUG=True,ROOT_URLCONF=__name__)
import django; django.setup()
from django.core.management import call_command
call_command('makemigrations','fixture_shop',verbosity=0); call_command('migrate',verbosity=0)
(app/'migrations/0002_note.py').write_text('from django.db import migrations,models\nclass Migration(migrations.Migration):\n dependencies=[("fixture_shop","0001_initial")]\n operations=[migrations.AddField("item","note",models.TextField(null=True))]\n')
for name,args,kwargs in [('migrations-plan.txt',['showmigrations'],{'format':'plan','verbosity':2}),('migration.sql',['sqlmigrate','fixture_shop','0002_note'],{})]:
 stream=io.StringIO(); call_command(*args,stdout=stream,**kwargs);(root/name).write_text(stream.getvalue(),encoding='utf-8')
from fixture_shop.models import Item
from django.db import connection,reset_queries
Item.objects.bulk_create([Item(name='item'+str(i)) for i in range(3)])
reset_queries()
for pk in list(Item.objects.values_list('pk',flat=True)): Item.objects.get(pk=pk)
(root/'queries.json').write_text(json.dumps([{'requestId':'fixture-request','sql':q['sql'],'durationMs':float(q['time'])*1000} for q in connection.queries]),encoding='utf-8')
from django.urls import path
from django.http import HttpResponse
def view(request,pk):return HttpResponse(str(pk))
urlpatterns=[path('items/<int:pk>/',view,name='detail')]
from django.urls import get_resolver
rows=[{'route':str(p.pattern),'name':p.name,'namespace':'','parameters':list(p.pattern.converters)} for p in get_resolver().url_patterns]
(root/'urls.json').write_text(json.dumps(rows),encoding='utf-8')
stream=io.StringIO();call_command('check',deploy=True,stdout=stream,stderr=stream);(root/'checks.txt').write_text(stream.getvalue(),encoding='utf-8')
print('PASS disposable Django migration/SQL/URL/check exports; version',django.get_version())
