"""Capture immutable reference cases; never load models or write to the reference tree."""
import os, sys, json, math, hashlib
from pathlib import Path
from datetime import datetime, timedelta
from dataclasses import asdict, replace
os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
sys.dont_write_bytecode = True
root = Path(sys.argv[1]).resolve()
sys.path.insert(0, str(root / 'src'))
import pyexiv2  # Reference requires this import before numerical/image libraries.
import numpy as np
from core.subject_grouping import (SubjectEvidence, SubjectArtifact, GeometryEvidence,
    verify_pair, generate_candidate_pairs, complete_link_clusters, PairVerification,
    _needs_geometry_verification)
from core.subject_grouping_models import HighAccuracySubjectModels, _dense_grid_evidence, _normalized, _descriptor_tuple
from core.app_settings import CullGroupingStrictness
rng = np.random.default_rng(930301)
def vec(n=8): return rng.normal(size=n).astype(np.float32).tolist()
def unit(c): return [c, math.sqrt(max(0, 1-c*c))]
def ev(v, box=(0.1,0.1,0.8,0.8), area=.49, kind='region'):
    return SubjectEvidence(tuple(v), tuple(box), area, kind)
def artifact(path, glob=(1.,0.), subjects=None, faces=()):
    return SubjectArtifact(path,(10,20),'fixture',tuple(glob),tuple(subjects if subjects is not None else [ev((1.,0.))]),tuple(faces))
def out_art(a):
    def out_ev(e):return dict(descriptor=e.descriptor,bbox=e.bbox,area=e.area_fraction,kind=e.kind)
    return dict(global_=a.global_descriptor,subjects=[out_ev(x) for x in a.subjects],faces=[out_ev(x) for x in a.faces])
def out_pair(p):
    d=asdict(p);d.pop('path_a');d.pop('path_b');d['complete']=d.pop('subject_set_complete');return d
stamp=lambda t: None if t is None else datetime(2020,1,1)+timedelta(milliseconds=t)
result={'source_sha256':{str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [root/'src/core/subject_grouping.py',root/'src/core/subject_grouping_models.py',root/'src/core/similarity_utils.py']},'pairs':[],'candidates':[],'clusters':[],'geometry':[]}
inputs=[]
for sim in [.0,.55,.61,.66,.72,.78,.82,.86,.90,.94,1.]:
 for scene in [.70,.79,.83,.95,1.]:
  for geom in [GeometryEvidence(),GeometryEvidence(8,.5,.12,.12,True),GeometryEvidence(7,.9,.5,.5,True)]:
   a=artifact('a'); b=artifact('b',unit(scene),[ev(unit(sim))])
   inputs.append((f'subject-{sim}-scene-{scene}-inliers-{geom.inlier_count}',a,b,geom,0,30000))
for i in range(35):
 n=i%6; a=artifact('a',vec(),[ev(vec(),area=.1+.07*j) for j in range(n)])
 b=artifact('b',vec(),[ev(vec(),area=.1+.09*j) for j in range(n+(i%11==0))])
 if i%2==0:a=replace(a,faces=(ev(vec(),kind='face'),));b=replace(b,faces=(ev(vec(),kind='face'),))
 inputs.append((f'assignment-{i}',a,b,GeometryEvidence(16,1,.5,.5,True),None if i%3==0 else 0,600000+i))
for label,a,b in [
 ('changed-face-count',artifact('a',faces=[ev([1,0],kind='face')]),artifact('b')),
 ('same-face',artifact('a',faces=[ev([1,0],kind='face')]),artifact('b',faces=[ev([1,0],kind='face')])),
 ('different-face',artifact('a',faces=[ev([1,0],kind='face')]),artifact('b',faces=[ev([0,1],kind='face')])),
 ('zero-descriptor',artifact('a',subjects=[ev([0,0])]),artifact('b')),
 ('different-dimensions',artifact('a',subjects=[ev([1,0,0])]),artifact('b')),
 ('small-norm',artifact('a',subjects=[ev([1e-6,0])]),artifact('b')),
 ('different-kind',artifact('a',subjects=[ev([1,0],kind='face')]),artifact('b')),
 ('empty-subjects',artifact('a',subjects=[]),artifact('b',subjects=[])),
]:inputs.append((label,a,b,GeometryEvidence(),None,None))
for name,a,b,g,ta,tb in inputs:
 for strict in CullGroupingStrictness:
  p=verify_pair(a,b,geometry=g,timestamp_a=stamp(ta),timestamp_b=stamp(tb),strictness=strict)
  result['pairs'].append(dict(name=name,first=out_art(a),second=out_art(b),strictness=strict.name.title(),geometry=asdict(g),time_a=ta,time_b=tb,expected=out_pair(p),needs_geometry=_needs_geometry_verification(p,strict)))
for i,n in enumerate([0,1,2,12,33,40,65,80,96]):
 for mode in ['random','ties','equal-time']:
  arts=[artifact(f'{j:04}',vec() if mode!='ties' else (1,0)) for j in range(n)]
  times=[None if j%7==0 else (0 if mode=='equal-time' else int(rng.integers(-900000,900000))) for j in range(n)]
  expected=generate_candidate_pairs({a.path:a for a in arts},{a.path:stamp(t) for a,t in zip(arts,times)})
  result['candidates'].append(dict(name=f'{mode}-{n}',artifacts=[out_art(a) for a in arts],times=times,expected=[[int(a),int(b)] for a,b in expected]))
for i in range(48):
 n=3+i%14; paths=[f'{j:04}' for j in range(n)]; pairs={}
 for a in range(n):
  for b in range(a+1,n):
   if rng.random()<.2:continue
   confidence=round(float(rng.uniform(.5,1)),2); accepted=bool(rng.random()<.75)
   pairs[(paths[a],paths[b])]=PairVerification(paths[a],paths[b],.9,None,.9,True,GeometryEvidence(),None,confidence,accepted,'fixture')
 groups=complete_link_clusters(list(reversed(paths)),pairs)
 result['clusters'].append(dict(count=n,pairs=[dict(a=int(a),b=int(b),pair=out_pair(p)) for (a,b),p in pairs.items()],expected=[groups[p] for p in paths]))
# Geometry descriptors make region correspondences unambiguous; transformed centers
# cover translation, perspective, rotation, degenerate and adversarial matches.
for mode in ['identity','translation','perspective','rotation','collinear','few','ties','outliers','noise']:
 for trial in range(3):
  n=3 if mode=='few' else 16
  subjects=[];other=[]
  for j in range(n):
   x=(j%4+.5)/4;y=(j//4+.5)/4
   if mode=='collinear':y=.5
   u,v=x,y
   if mode=='translation':u+=.04;v+=.08
   if mode=='perspective':u=(x+.1*y)/(1+.15*x);v=y/(1+.15*x)
   if mode=='rotation':u=1-y;v=x
   if mode=='outliers' and j%5==trial:u,v=rng.uniform(0,1,2)
   if mode=='noise':u+=float(rng.normal(0,.035));v+=float(rng.normal(0,.035))
   descriptor=[float(k==j) for k in range(16)] if mode!='ties' else [1.,0.]
   subjects.append(ev(descriptor,(x-.025,y-.025,x+.025,y+.025),.0025))
   other.append(ev(descriptor,(u-.025,v-.025,u+.025,v+.025),.0025))
  a=artifact('a',subjects=subjects);b=artifact('b',subjects=other)
  g=HighAccuracySubjectModels.verify_geometry(None,a,b)
  result['geometry'].append(dict(name=f'{mode}-{trial}',first=out_art(a),second=out_art(b),expected=asdict(g),strong=g.strong))
result['dense'] = []
for count in [1,4,9,16,25,64,256,6]:
 for mode in ['random','zero']:
  tokens = rng.normal(size=(count+1,8)).astype(np.float32) if mode=='random' else np.zeros((count+1,8),dtype=np.float32)
  regions = _dense_grid_evidence(_normalized(tokens[1:]))
  a = artifact('dense',_descriptor_tuple(_normalized(tokens[0])),regions)
  result['dense'].append(dict(name=f'{mode}-{count}',tokens=tokens.reshape(-1).tolist(),width=8,expected=out_art(a)))
# Rust's JSON field name is the keyword-safe global, independent of Python naming.
raw=json.dumps(result,separators=(',',':')).replace('"global_":','"global":')
Path(sys.argv[2]).write_text(raw+'\n')
print({k:len(v) for k,v in result.items()})
