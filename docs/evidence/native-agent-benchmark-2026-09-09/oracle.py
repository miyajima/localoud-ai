import random,copy,json
from intervals import normalize,subtract,contains
rng=random.Random(20260909)
points=[x/2 for x in range(-24,25)]
def make():
 return [tuple(sorted((rng.randint(-10,10),rng.randint(-10,10)))) for _ in range(rng.randrange(9))]
def member(xs,p): return any(a<=p<b for a,b in xs)
def canonical(xs):
 assert all(a<b for a,b in xs)
 assert all(xs[i][1]<xs[i+1][0] for i in range(len(xs)-1))
for case in range(2000):
 xs,cuts=make(),make(); original=copy.deepcopy((xs,cuts))
 n=normalize(xs);s=subtract(xs,cuts);canonical(n);canonical(s)
 for p in points:
  assert member(n,p)==member(xs,p),(case,'normalize')
  assert contains(xs,p)==member(xs,p),(case,'contains')
  assert member(s,p)==(member(xs,p) and not member(cuts,p)),(case,'subtract')
 assert (xs,cuts)==original,(case,'mutation')
for f,args in [(normalize,([(2,1)],)),(subtract,([(2,1)],[])),(subtract,([] ,[(2,1)])),(contains,([(2,1)],0))]:
 try: f(*args)
 except ValueError: pass
 else: raise AssertionError('reversed bounds not rejected')
print(json.dumps({'pass':True,'random_cases':2000,'points_per_case':len(points),'seed':20260909,'reversed_checks':4}))
