"""Capture the reference's neutral inputs without writing to the Python checkout."""
import hashlib, json, sys, shutil
from pathlib import Path
sys.dont_write_bytecode = True
reference, output = (Path(p).resolve() for p in sys.argv[1:3])
output.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(reference / 'src'))
import rawpy, pyexiv2, cv2
import numpy as np
from PIL import Image
from core.image_processing.raw_image_processor import RawImageProcessor
from core.image_processing.standard_image_processor import StandardImageProcessor
from workers.easy_delete_worker import EasyDeleteWorker
cases=[]
def measurements(image):
    rgb=np.asarray(image.convert('RGB'))
    worker=EasyDeleteWorker([],image_pipeline=object())
    worker._get_analysis_rgb=lambda _:rgb
    gray=worker._load_gray_for_detection('fixture')
    whole=cv2.cvtColor(rgb,cv2.COLOR_RGB2GRAY)
    return dict(sharpness=worker._compute_local_sharpness(gray),quality_sharpness=float(cv2.Laplacian(whole,cv2.CV_64F).var()),brightness=float(gray.mean()))
for path in sorted((reference/'tests/samples').iterdir()):
    if not path.is_file(): continue
    before=hashlib.sha256(path.read_bytes()).hexdigest()
    image=(RawImageProcessor.load_raw_for_blur_detection(str(path),target_size=(1024,1024),apply_auto_edits=False)
           if path.suffix.lower()=='.arw' else StandardImageProcessor.load_for_blur_detection(str(path),target_size=(1024,1024)))
    assert image is not None
    image.save(output/(path.name+'.png'))
    assert hashlib.sha256(path.read_bytes()).hexdigest()==before
    cases.append(dict(name=path.name,sha256=before,size=image.size, measurements=measurements(image)))
# Remove only preview pointer tags from an independent RAW copy to exercise
# LibRaw's fallback. This copy is disposable; all seven sources stay untouched.
fallback=output/'no-preview.ARW'
shutil.copy2(reference/'tests/samples/arw_sample.ARW',fallback)
with pyexiv2.Image(str(fallback)) as im:
    tags=im.read_exif()
    im.modify_exif({key:None for key in tags if 'JPEGInterchangeFormat' in key})
with rawpy.imread(str(fallback)) as raw:
    try: raw.extract_thumb()
    except (rawpy.LibRawNoThumbnailError,rawpy.LibRawUnsupportedThumbnailError): pass
    else: raise AssertionError('RAW fixture still has a JPEG preview')
image=RawImageProcessor.load_raw_for_blur_detection(str(fallback),target_size=(1024,1024),apply_auto_edits=False)
assert image is not None
image.save(output/'no-preview.ARW.png')
(output/'reference.json').write_text(json.dumps(dict(cases=cases, fallback_size=image.size,
    rawpy_version=rawpy.__version__,libraw_version=rawpy.libraw_version,
    source_sha256={str(p.relative_to(reference)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [reference/'src/core/image_pipeline.py',reference/'src/core/image_processing/raw_image_processor.py',reference/'src/core/image_processing/standard_image_processor.py']}),indent=2)+'\n')
print('Captured seven neutral inputs and RAW fallback:',output)

fixture=Path(__file__).resolve().parent.parent/'tests/fixtures/analysis/palette.png'
fixture.parent.mkdir(parents=True, exist_ok=True)
pattern=Image.new('P',(2048,2))
pattern.putpalette([0,0,0,255,255,255]+[0]*762)
pattern.putdata([x%2 for y in range(2) for x in range(2048)])
pattern.save(fixture)
nearest=pattern.copy();nearest.thumbnail((1024,1024),Image.Resampling.LANCZOS)
print('Palette reference thumbnail unique values:',np.unique(np.asarray(nearest.convert('RGB')).reshape(-1,3),axis=0).tolist())
