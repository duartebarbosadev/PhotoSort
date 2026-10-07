import sys,json
from pathlib import Path
sys.dont_write_bytecode=True
sys.path.insert(0,str(Path(sys.argv[1]).resolve()/'src'))
import pyexiv2
import numpy as np
from PIL import Image
from core.subject_grouping_models import _aligned_face_crop
pixels=np.asarray([[(x*7%256,y*11%256,(x*13+y*17)%256) for x in range(32)] for y in range(24)],dtype=np.uint8)
image=Image.fromarray(pixels)
cases=[]
for name,bbox,eyes in [
 ('level',[.25,.25,.75,.75],[[.375,.375],[.625,.375]]),
 ('tilt-left',[.25,.25,.75,.75],[[.375,.5],[.625,.375]]),
 ('tilt-right',[.25,.25,.75,.75],[[.375,.375],[.625,.5]]),
 ('vertical',[.25,.25,.75,.75],[[.5,.375],[.5,.625]]),
 ('upside-down',[.25,.25,.75,.75],[[.625,.5],[.375,.5]]),
 ('left-edge',[-.125,.125,.375,.875],[[.0,.5],[.25,.375]]),
 ('right-edge',[.625,.125,1.125,.875],[[.75,.375],[1.,.5]]),
]:
 landmarks=[(0.,0.)]*478;landmarks[33]=eyes[0];landmarks[263]=eyes[1]
 out=_aligned_face_crop(image,tuple(bbox),landmarks)
 cases.append(dict(name=name,face=dict(bbox=bbox,eyes=eyes),width=out.width,height=out.height,pixels=list(out.tobytes())))
Path(sys.argv[2]).write_text(json.dumps(cases,separators=(',',':')))
print('Captured',len(cases),'aligned crop cases')
