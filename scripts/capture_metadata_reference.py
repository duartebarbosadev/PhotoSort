"""Read the untouched reference and create small native-metadata regression fixtures."""
import hashlib, json, sys
from pathlib import Path
sys.dont_write_bytecode = True
reference, output = (Path(p).resolve() for p in sys.argv[1:3])
sys.path.insert(0, str(reference / 'src'))
import pyexiv2
from PIL import Image
from core.pyexiv2_wrapper import PyExiv2Operations
from workers.easy_delete_worker import EasyDeleteWorker
output.mkdir(parents=True, exist_ok=True)
cases=[]
def capture(path, sample=False):
    data=PyExiv2Operations.get_comprehensive_metadata(str(path))
    class Cache:
        def get(self, _): return data
    worker=EasyDeleteWorker([],image_pipeline=object(),exif_disk_cache=Cache())
    cases.append(dict(name=path.name,sample=sample,fields=worker._exif_field_count(str(path)),
        sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
        camera=data.get('Exif.Image.Model',''),lens=data.get('Exif.Photo.LensModel',''),
        iso=data.get('Exif.Photo.ISOSpeedRatings',''), orientation=int(data.get('Exif.Image.Orientation',1))))
for path in sorted((reference/'tests/samples').iterdir()):
    if path.is_file():capture(path,True)
for name in ['empty.jpg','mixed.jpg','empty-values.jpg','gps.jpg']:
    path=output/name
    Image.new('RGB',(36,24),(84,110,140)).save(path)
    if name=='mixed.jpg':
        with pyexiv2.Image(str(path)) as image:
            image.modify_exif({'Exif.Image.Model':'Fixture Camera','Exif.Photo.LensModel':'Fixture Lens','Exif.Photo.ISOSpeedRatings':'200','Exif.Image.Orientation':'6','Exif.Photo.DateTimeOriginal':'2024:02:03 04:05:06','Exif.Photo.FNumber':'28/10','Exif.Photo.ExposureTime':'1/125'})
            image.modify_iptc({'Iptc.Application2.Keywords':['test','metadata'],'Iptc.Application2.Caption':'Photo'})
            image.modify_xmp({'Xmp.xmp.Rating':'4','Xmp.xmp.CreateDate':'2024-01-01T00:00:00Z','Xmp.dc.subject':['a','b']})
    if name=='empty-values.jpg':
        with pyexiv2.Image(str(path)) as image:
            image.modify_exif({'Exif.Image.Artist':'','Exif.Image.Copyright':'None','Exif.Image.ImageDescription':' ','Exif.Photo.ISOSpeedRatings':'0'})
            image.modify_xmp({'Xmp.xmp.Label':'','Xmp.xmp.Rating':'0','Xmp.dc.subject':[],'Xmp.dc.creator':[''],'Xmp.dc.rights':{'lang="x-default"':''}})
    if name=='gps.jpg':
        with pyexiv2.Image(str(path)) as image:
            image.modify_exif({'Exif.GPSInfo.GPSLatitude':'31/1 57/1 0/1','Exif.GPSInfo.GPSLatitudeRef':'S','Exif.GPSInfo.GPSLongitude':'115/1 52/1 12/1','Exif.GPSInfo.GPSLongitudeRef':'E'})
    capture(path)
result=dict(pyexiv2_version=pyexiv2.__version__,exiv2_version=pyexiv2.__exiv2_version__,source_sha256={str(p.relative_to(reference)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [reference/'src/core/pyexiv2_wrapper.py',reference/'src/workers/easy_delete_worker.py']},cases=cases)
(output/'reference.json').write_text(json.dumps(result,indent=2)+'\n')
print([(c['name'],c['fields']) for c in cases])
