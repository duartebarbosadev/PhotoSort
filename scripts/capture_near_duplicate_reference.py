"""Capture decode/model-independent evidence from the read-only Python reference."""
import sys, os, json, hashlib, math
from pathlib import Path
from dataclasses import asdict
sys.dont_write_bytecode = True
os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
root = Path(sys.argv[1]).resolve()
sys.path.insert(0, str(root / 'src'))
import pyexiv2  # Match the reference environment's native import order.
import numpy as np
from core.image_features.near_duplicate import coherent_change_metrics, _exposure_correct, _validate_alignment
from core.image_features.structural_similarity import structural_similarity_for_aligned

def arrays(c):
    w,h=c['width'],c['height']
    i=np.arange(w*h,dtype=np.uint64)
    if c.get('flat') is not None:
        first=np.full((h,w), c['flat'],dtype=np.float32)
    else:
        first=(((i*73+(i//w)*37+c['seed']*19)%181+32).astype(np.float32)/np.float32(255)).reshape(h,w)
    second=np.clip(first*np.float32(c['gain'])+np.float32(c['offset']),0,1)
    for x,y,rw,rh,d in c['patches']:
        second[y:y+rh,x:x+rw]=np.clip(second[y:y+rh,x:x+rw]+np.float32(d),0,1)
    mask=np.ones((h,w),dtype=bool)
    for x,y,rw,rh in c['holes']:mask[y:y+rh,x:x+rw]=False
    return first,second,mask

cases=[]
def add(name,**kwargs):
    c=dict(name=name,width=128,height=96,seed=3,gain=1.0,offset=0.0,patches=[],holes=[],normal=False,masked=True)
    c.update(kwargs);cases.append(c)
add('identical')
add('uniform-exposure',gain=1.03,offset=.012)
add('flat-identical',flat=.4)
add('gain-too-large',gain=.4)
add('offset-too-large',gain=.6,offset=.32)
for edge in [2,3,5,6,7,8,12,24]:
    add(f'component-{edge}',patches=[[30,30,edge,edge,.2]])
    add(f'normal-component-{edge}',patches=[[30,30,edge,edge,.2]],normal=True)
for width,height in [(2,30),(5,40),(6,24),(6,25),(12,12)]:
    add(f'normal-shape-{width}-{height}',patches=[[35,20,width,height,.13]],normal=True)
for x,y in [(0,0),(120,0),(0,88),(120,88),(60,42)]:
    add(f'border-{x}-{y}',patches=[[x,y,8,8,.18]])
add('masked-change',patches=[[20,20,20,20,.2]],holes=[[15,15,30,30]])
add('masked-border',patches=[[20,20,20,20,.2]],holes=[[0,0,28,96]])
add('mask-support-erased',holes=[[0,0,128,45],[0,51,128,45]])
add('empty-mask',holes=[[0,0,128,96]])
add('many-components',patches=[[x,y,7,7,.1] for y in range(8,80,15) for x in range(8,110,15)])
add('diagonal-connected',patches=[[20+i*6,20+i*6,7,7,.12] for i in range(7)])
add('close-gap',patches=[[20,20,8,15,.1],[31,20,8,15,.1]])
add('low-contrast',patches=[[20,20,40,40,.045]])
add('under-threshold',patches=[[20,20,40,40,.03]])
add('changed-scene',gain=.7,patches=[[0,0,64,96,.2]])
for w,h in [(1,1),(2,3),(3,2),(5,7),(33,29),(127,99),(256,192)]:
    add(f'irregular-{w}-{h}',width=w,height=h,masked=False,gain=.99,offset=.003)
result={'source_sha256':{},'signals':cases,'alignment':[]}
for rel in ['src/core/image_features/near_duplicate.py','src/core/image_features/structural_similarity.py']:
    result['source_sha256'][rel]=hashlib.sha256((root/rel).read_bytes()).hexdigest()
for c in cases:
    a,b,m=arrays(c);mask=m if c['masked'] else None
    opts=dict(component_min_fraction=.0008,component_min_pixels=24,combined_min_fraction=.0025,component_min_thickness=6,component_max_elongation=4.0,exposure_correct=False) if c['normal'] else {}
    change=coherent_change_metrics(a,b,valid_mask=mask,**opts)
    c['change']=asdict(change) if change is not None else None
    c['ssim']=structural_similarity_for_aligned(a*255,b*255,mask)
    exposure=_exposure_correct(a,b,mask)
    c['exposure']=None if exposure is None else [float(exposure.flat[i]) for i in sorted(set([0,a.size//4,a.size//2,a.size-1]))]

def alignment(name,t,corr=.995,w=1000,h=800):
    v=_validate_alignment(np.asarray(t,dtype=np.float64),correlation=corr,width=w,height=h)
    result['alignment'].append(dict(name=name,transform=t,correlation=corr,width=w,height=h,expected=asdict(v) if v else None))
for corr in [.0,.899999,.9,.994999,.995,1.0]:alignment(f'correlation-{corr}',[[1,0,0],[0,1,0]],corr)
for x,y in [(40,32),(-40,-32),(40.0001,0),(0,-32.0001),(0,0)]:alignment(f'translation-{x}-{y}',[[1,0,x],[0,1,y]])
for deg in [-1.5001,-1.5,-1.4999,0,1.4999,1.5,1.5001]:
    a=math.radians(deg);alignment(f'rotation-{deg}',[[math.cos(a),-math.sin(a),2],[math.sin(a),math.cos(a),-3]])
for scale in [.979999,.98,.980001,1.,1.019999,1.02,1.020001]:alignment(f'scale-{scale}',[[scale,0,0],[0,scale,0]])
for shear in [.0099,.01,.0101]:alignment(f'shear-{shear}',[[1,shear,0],[0,1,0]])
alignment('degenerate',[[0,0,0],[0,0,0]])
# Isolate face-motion evidence: crop SSIM has its own signal fixtures above.
# No detector/model is initialized by these fixtures.
from core.image_features.face_analysis import FaceDescriptor, SubjectDescriptor
from core.image_features import near_duplicate as nd
nd._face_crop_similarity = lambda *args: 1.0
result['faces'] = []
def face(x=.2,y=.2,w=.25,h=.3):
    points=np.array([(x+w*i/6,y+h*j/6) for j in range(7) for i in range(7)],dtype=np.float32)
    return make_face(points)
def make_face(points):
    points=np.asarray(points,dtype=np.float32)
    return FaceDescriptor(tuple(float(v) for v in [points[:,0].min(),points[:,1].min(),points[:,0].max(),points[:,1].max()]),tuple(tuple(float(v) for v in p) for p in points))
def move(f,dx=0,dy=0):return make_face(np.asarray(f.landmarks)+[dx,dy])
def mapped(f,t):
    pts=np.asarray(f.landmarks)*[1000,800]
    return make_face((pts@np.asarray(t)[:,:2].T+np.asarray(t)[:,2])/[1000,800])
def face_case(name,a,b,t=None):
    t=t or [[1,0,0],[0,1,0]]
    actual=nd._compare_faces(SubjectDescriptor(tuple(a)),SubjectDescriptor(tuple(b)),np.empty((800,1000),dtype=np.float32),None,None,np.asarray(t,dtype=np.float64))
    def encode(f):return dict(bbox=f.bbox,landmarks=f.landmarks,eyes=[f.landmarks[0],f.landmarks[-1]])
    result['faces'].append(dict(name=name,first=list(map(encode,a)),second=list(map(encode,b)),transform=t,decision=actual['decision'].value,min_iou=actual['min_iou'],max_median=actual['max_median'],max_p90=actual['max_p90']))
f=face();g=face(.65,.3,.2,.25)
face_case('no-faces',[],[])
face_case('count-changed',[f],[])
face_case('same',[f],[f])
face_case('reordered',[f,g],[g,f])
for fraction in [0,.10,.24,.25,.26,.6]:face_case(f'face-shift-{fraction}',[f],[move(f,dx=.25*fraction)])
for displacement in [.005,.015,.03,.05,.08]:
    pts=np.asarray(f.landmarks);pts[10:20,1]+=displacement
    face_case(f'expression-{displacement}',[f],[make_face(pts)])
for deg in [-1.4,0,1.4]:
    r=math.radians(deg);t=[[math.cos(r),-math.sin(r),7],[math.sin(r),math.cos(r),-4]]
    face_case(f'camera-rotation-{deg}',[f,g],[mapped(g,t),mapped(f,t)],t)
face_case('incompatible-landmarks',[f],[FaceDescriptor(f.bbox,f.landmarks[:-1])])
face_case('two-face-one-changed',[f,g],[f,move(g,.07)])
face_case('overlap-tie',[f,f],[f,f])
Path(sys.argv[2]).write_text(json.dumps(result,indent=2,allow_nan=False)+'\n')
print(f"Captured {len(cases)} signal and {len(result['alignment'])} alignment cases and {len(result['faces'])} face-motion cases")
