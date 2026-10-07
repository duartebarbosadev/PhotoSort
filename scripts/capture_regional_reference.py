"""Capture region boxes/distances/DBSCAN results; reference checkout stays read-only."""
import sys,json,hashlib
from pathlib import Path
sys.dont_write_bytecode=True
root=Path(sys.argv[1]).resolve();out=Path(sys.argv[2]);sys.path.insert(0,str(root/'src'))
import numpy as np
from sklearn.cluster import DBSCAN
from core.similarity_utils import build_regional_distance_matrix
from core.similarity_embedding_model import build_similarity_image_regions
class Probe:
 def __init__(self,w,h):self.size=(w,h);self.boxes=[]
 def crop(self,box):self.boxes.append(box);return box
boxes=[]
for w in [1,2,3,5,10,17,31,100,101,640,1024,2048]:
 for h in [1,3,5,11,100,101,480,1024]:
  p=Probe(w,h);build_similarity_image_regions(p);boxes.append(dict(width=w,height=h,boxes=p.boxes))
rng=np.random.default_rng(311)
cases=[]
def add(name,rows,orient=None,eps=.055):
 paths=[str(i) for i in range(len(rows))];emb={p:r[0] for p,r in zip(paths,rows)};regions=dict(zip(paths,rows))
 matrix=build_regional_distance_matrix(emb,regions,paths)
 portraits=orient if orient is not None else [False]*len(rows)
 labels=[0]*len(rows);next_id=1
 for portrait in [True,False]:
  ids=[i for i,p in enumerate(portraits) if p==portrait]
  if not ids:continue
  found=DBSCAN(eps=eps,min_samples=2,metric='precomputed').fit_predict(matrix[np.ix_(ids,ids)])
  mapping={label:next_id+i for i,label in enumerate(sorted(set(found)-{-1}))};next_id+=len(mapping)
  for i,label in zip(ids,found):
   if label==-1:labels[i]=next_id;next_id+=1
   else:labels[i]=mapping[label]
 cases.append(dict(name=name,rows=rows,portrait=portraits,eps=eps,distances=matrix.tolist(),labels=labels))
def angle(deg):
 r=np.deg2rad(deg);return [float(np.cos(r)),float(np.sin(r))]
add('chain-keeps-dbscan-transitivity',[[angle(d)]*6 for d in [0,15,30,100]])
add('clusters-before-noise',[[angle(d)]*6 for d in [100,0,10,110,230]])
add('orientation-splits-identical',[[[1.,0.]]*6]*5,[False,True,False,True,True])
add('zero-vectors-are-not-identical-neighbors',[[[0.,0.]]*6]*3)
add('same-background-different-ordered-regions',[[angle(0)]*6, [angle(0)]+[angle(90)]*5])
add('legacy-unequal-region-best-pair',[[angle(0)]*6,[angle(0),angle(90)],[angle(90)]])
add('one-zero-row-blocks-deduplication',[[[0.,0.]]+[[1.,0.]]*5]*3)
for n in range(35):
 rows=[];count=4+n%13;width=4+n%5
 for i in range(count):
  if i>0 and i%4==0:rows.append(rows[-1]);continue
  base=np.zeros(width,dtype=np.float32);base[i%3]=1
  noise=rng.normal(0,.08+(n%4)*.025,(6,width)).astype(np.float32)
  value=(base+noise).tolist()
  if n%7==0 and i%3==0:value=value[:1]
  rows.append(value)
 add(f'seeded-{n}',rows,[i%3==0 for i in range(count)], [.02,.055,.1,.2][n%4])
for n,scale in enumerate([.003,.008,.015,.025,.04,.07]):
 rows=[]
 for i in range(8):
  base=np.zeros(384,dtype=np.float32);base[i%2]=1
  rows.append((base+rng.normal(0,scale,(6,384)).astype(np.float32)).tolist())
 add(f'dino-width-{n}',rows,[i%3==0 for i in range(8)],.055)
result=dict(source_sha256={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [root/'src/core/similarity_utils.py',root/'src/core/similarity_embedding_model.py',root/'src/core/similarity_engine.py']},boxes=boxes,cases=cases)
out.write_text(json.dumps(result,indent=2,allow_nan=False)+'\n')
print(len(boxes),'crop-box cases;',len(cases),'distance/clustering cases')
