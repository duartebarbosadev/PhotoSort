"""Capture measurements from the untouched Python worker at the shared RGB boundary."""
import sys, json, hashlib
from pathlib import Path
sys.dont_write_bytecode = True
reference=Path(sys.argv[1]).resolve()
out=Path(sys.argv[2])
sys.path.insert(0,str(reference/'src'))
import pyexiv2
import cv2
import numpy as np
from workers.easy_delete_worker import EasyDeleteWorker
from core import app_settings
app_settings.get_easy_delete_blur_threshold=lambda:100.
app_settings.get_easy_delete_dark_threshold=lambda:15.
app_settings.get_easy_delete_white_threshold=lambda:248.
worker=EasyDeleteWorker([],image_pipeline=object())
cases=[]; binary=bytearray()
def add(w,h,kind,seed=3):
 i=np.arange(w*h,dtype=np.uint64).reshape(h,w)
 if kind=='noise': rgb=np.stack([(i*73+(i//w)*37+seed*19+c*47)%256 for c in range(3)],axis=-1).astype(np.uint8)
 elif kind=='gray':
  a=((i*73+(i//w)*37+seed*19)%256).astype(np.uint8);rgb=np.repeat(a[:,:,None],3,axis=2)
 elif kind=='chess':
  a=(127+(i%w+i//w)%2*2).astype(np.uint8);rgb=np.repeat(a[:,:,None],3,axis=2)
 elif kind=='night':
  a=(i%17).astype(np.uint8);rgb=np.repeat(a[:,:,None],3,axis=2)
 elif kind=='flat': rgb=np.full((h,w,3),seed,dtype=np.uint8)
 elif kind=='tiles':
  a=((i%w>=w//2)*100+50).astype(np.uint8);rgb=np.repeat(a[:,:,None],3,axis=2)
 worker._get_analysis_rgb=lambda _:rgb
 gray=worker._load_gray_for_detection('fixture')
 issue=worker._detect_issue('fixture')
 whole=cv2.cvtColor(rgb,cv2.COLOR_RGB2GRAY)
 c=dict(name=f'{kind}-{w}x{h}-{seed}',width=w,height=h,kind=kind,seed=seed,
  detection_width=gray.shape[1],detection_height=gray.shape[0],
  offset=len(binary),length=gray.size,sharpness=worker._compute_local_sharpness(gray),
  quality_sharpness=float(cv2.Laplacian(whole,cv2.CV_64F).var()),brightness=float(gray.mean()),
  black_fraction=float(np.count_nonzero(gray<=10)/gray.size),issue=issue['type'] if issue else None)
 binary.extend(gray.tobytes());cases.append(c)
for w,h in [(1,1),(1,7),(7,1),(2,3),(3,2),(3,3),(4,4),(5,7),(7,5),(11,13),(65,49)]:
 for kind in ['noise','gray','chess','night','tiles']:add(w,h,kind)
for w,h in [(640,480),(641,480),(640,481),(1001,1000),(1024,683),(683,1024),(960,720),(720,960),(1024,768),(512,1024),(1024,1),(1,1024),(960,480),(640,960)]:
 for kind in ['noise','chess','night']:add(w,h,kind)
for value in [0,10,12,15,128,249,255]: add(17,19,'flat',value)
result=dict(opencv_version=cv2.__version__,source_sha256={str(p.relative_to(reference)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [reference/'src/workers/easy_delete_worker.py',reference/'src/core/app_settings.py',reference/'src/core/best_photo_finder/scorers.py']},cases=cases)
out.write_text(json.dumps(result,indent=2,allow_nan=False)+'\n')
out.with_suffix('.bin').write_bytes(binary)
print(f'{len(cases)} cases; {len(binary)} grayscale bytes')
