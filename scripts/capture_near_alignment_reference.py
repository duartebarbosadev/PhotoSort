"""Capture complete comparator results and immutable synthetic PNG pairs."""
import sys,os,json,hashlib,runpy
from pathlib import Path
from dataclasses import asdict
sys.dont_write_bytecode=True
os.environ['PYTHONDONTWRITEBYTECODE']='1'
root=Path(sys.argv[1]).resolve();out=Path(sys.argv[2]);out.mkdir(parents=True,exist_ok=True)
sys.path.insert(0,str(root/'src'))
import pyexiv2
import numpy as np
import cv2
from core.image_features.near_duplicate import SubjectSafeNearDuplicateComparator
from core.image_features.face_analysis import SubjectDescriptor
helpers=runpy.run_path(str(root/'tests/test_subject_safe_near_duplicate.py'))
scene=helpers['_high_contrast_scene']
handheld=helpers['_handheld_affine']
no_faces=SubjectDescriptor(())
cases=[]
def gray(image):return cv2.cvtColor(image,cv2.COLOR_RGB2GRAY) if image.ndim==3 else image
images={}
def add(name,a,b,suite='small',faces_a=no_faces,faces_b=no_faces):
    a,b=gray(a),gray(b)
    paths=[]
    for data in [a,b]:
        digest=hashlib.sha256(data.tobytes()).hexdigest()
        path=images.get(digest)
        if path is None:
            path=f'{len(images):02d}.png';cv2.imwrite(str(out/path),data);images[digest]=path
        paths.append(path)
    r=SubjectSafeNearDuplicateComparator().assess('a','b',None,None,a,b,descriptor_a=faces_a,descriptor_b=faces_b)
    cases.append(dict(name=name,first=paths[0],second=paths[1],suite=suite,decision=r.decision.value,reason=r.reason_code,structural=r.structural_similarity,change=asdict(r.change) if r.change else None,alignment=r.metrics.get('alignment_transform'),correlation=r.metrics.get('alignment_correlation'),overlap=r.metrics.get('alignment_overlap_fraction'),faces_a=None if faces_a is None else [dict(bbox=f.bbox,landmarks=f.landmarks,eyes=[f.landmarks[0],f.landmarks[-1]]) for f in faces_a.faces],faces_b=None if faces_b is None else [dict(bbox=f.bbox,landmarks=f.landmarks,eyes=[f.landmarks[0],f.landmarks[-1]]) for f in faces_b.faces]))
base=scene()
small=cv2.resize(base,(128,96),interpolation=cv2.INTER_AREA)
add('identical',small,small)
add('small-camera-motion',small,handheld(small))
add('exposure',small,np.clip(small.astype(np.float32)*1.03+3,0,255).astype(np.uint8))
changed=small.copy();changed[35:55,40:70]=255-changed[35:55,40:70]
add('changed-scene',small,changed)
add('missing-faces',small,small,faces_a=None,faces_b=None)
f=helpers['_face']()
add('face-count-changed',small,small,faces_a=SubjectDescriptor((f,)))
points=list(f.landmarks);points[-20:]=[(x,y+.04) for x,y in points[-20:]]
from core.image_features.face_analysis import FaceDescriptor
add('face-expression',small,small,faces_a=SubjectDescriptor((f,)),faces_b=SubjectDescriptor((FaceDescriptor(f.bbox,tuple(points)),)))
add('stable-face',small,small,faces_a=SubjectDescriptor((f,)),faces_b=SubjectDescriptor((f,)))
face_changed=small.copy();face_changed[40:45,57:64]=20
add('face-crop-change',small,face_changed,faces_a=SubjectDescriptor((f,)),faces_b=SubjectDescriptor((f,)))
tiny=FaceDescriptor((.4,.4,.41,.41),((.4,.4),(.41,.4),(.41,.41),(.4,.41)))
add('face-crop-too-small',small,small,faces_a=SubjectDescriptor((tiny,)),faces_b=SubjectDescriptor((tiny,)))

flat=np.full((96,128),100,dtype=np.uint8);add('flat',flat,flat)
for shift in [3,8]:
    moved=cv2.warpAffine(small,np.float32([[1,0,shift],[0,1,0]]),(128,96),borderMode=cv2.BORDER_REFLECT)
    add(f'translation-{shift}',small,moved)
add('normal-handheld-scene',base,handheld(base),'large')
add('normal-body-pose',base,handheld(scene(1)),'large')
textured=helpers['_textured_rgb']();changed=textured.copy();changed[550:590,450:470]=(220,30,30)
add('normal-small-subject',textured,changed,'large')
rng=np.random.default_rng(7)
noise=np.clip(textured.astype(np.float32)*1.03+3+rng.normal(0,.8,textured.shape),0,255).astype(np.uint8)
add('normal-sensor-noise',textured,noise,'large')
result=dict(source_sha256={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [root/'src/core/image_features/near_duplicate.py',root/'tests/test_subject_safe_near_duplicate.py']},opencv=cv2.__version__,cases=cases)
(out/'reference.json').write_text(json.dumps(result,indent=2,allow_nan=False)+'\n')
for c in cases:print(c['name'],c['decision'],c['reason'],c['structural'],c['correlation'])
print(f'{len(images)} PNGs, {sum(p.stat().st_size for p in out.iterdir())} bytes')

# Isolate resampling from registration optimization to localize numerical drift.
from core.image_features.near_duplicate import _normal_view_pair
from core.image_features.structural_similarity import prepare_same_frame_preview
c=next(c for c in cases if c['name']=='normal-handheld-scene')
a=cv2.imread(str(out/c['first']),cv2.IMREAD_GRAYSCALE).astype(np.float32)/255
b=cv2.imread(str(out/c['second']),cv2.IMREAD_GRAYSCALE).astype(np.float32)/255
normal,_,_= _normal_view_pair(a,a,np.ones(a.shape,dtype=bool))
preview=prepare_same_frame_preview(normal*255)/255
normal.astype('<f4').tofile(out/'normal.f32')
preview.astype('<f4').tofile(out/'preview.f32')
t=np.array(c['alignment'],dtype=np.float64)
warped=cv2.warpAffine(b,t,(a.shape[1],a.shape[0]),flags=cv2.INTER_LINEAR|cv2.WARP_INVERSE_MAP,borderMode=cv2.BORDER_CONSTANT)
mask=cv2.warpAffine(np.ones(a.shape,dtype=np.uint8),t,(a.shape[1],a.shape[0]),flags=cv2.INTER_NEAREST|cv2.WARP_INVERSE_MAP,borderMode=cv2.BORDER_CONSTANT)
indices=sorted(set(list(range(0,a.size,7919))+[0,a.shape[1]-1,a.size//2,a.size-1]))
(out/'stages.json').write_text(json.dumps(dict(first=c['first'],second=c['second'],transform=c['alignment'],width=a.shape[1],height=a.shape[0],normal_width=normal.shape[1],normal_height=normal.shape[0],valid_count=int(mask.sum()),probes=[dict(index=i,value=float(warped.flat[i]),valid=bool(mask.flat[i])) for i in indices]),indent=2)+'\n')
from core.image_features.near_duplicate import _exposure_correct
corrected=_exposure_correct(a,warped,mask.astype(bool))
normal_a,normal_b,normal_mask=_normal_view_pair(a,corrected,mask.astype(bool))
preview_b=prepare_same_frame_preview(normal_b*255)/255
preview_mask=cv2.resize(normal_mask.astype(np.uint8),(128,96),interpolation=cv2.INTER_NEAREST)
normal_b.astype('<f4').tofile(out/'normal_second.f32')
preview_b.astype('<f4').tofile(out/'preview_second.f32')
cv2.imwrite(str(out/'valid_mask.png'),mask*255)
cv2.imwrite(str(out/'normal_mask.png'),normal_mask.astype(np.uint8)*255)
cv2.imwrite(str(out/'preview_mask.png'),preview_mask*255)
warped.astype('<f4').tofile(out/'warped.f32')
corrected.astype('<f4').tofile(out/'corrected.f32')
