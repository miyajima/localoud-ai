import random,itertools,copy,json
from scheduler import topological_order,execution_batches,critical_path_length
rng=random.Random(202609091)
for case in range(1000):
 n=rng.randrange(7);ids=list('abcdef'[:n]);rng.shuffle(ids);tasks={}
 for i,node in enumerate(ids):
  deps=[v for v in ids[:i] if rng.random()<0.3]
  if deps and rng.random()<0.3:deps.append(deps[0])
  tasks[node]={'deps':deps,'duration':rng.randrange(1,10),'group':rng.choice([None,None,'x','y'])}
 items=list(tasks.items());rng.shuffle(items);tasks=dict(items);before=copy.deepcopy(tasks)
 valid=[]
 for order in itertools.permutations(sorted(tasks)):
  pos={v:i for i,v in enumerate(order)}
  if all(pos[d]<pos[v] for v in tasks for d in tasks[v]['deps']):valid.append(order)
 assert topological_order(tasks)==list(min(valid)),(case,'topological')
 for capacity in [1,2,3,7]:
  done=set();expected=[]
  while len(done)<n:
   ready=sorted(v for v in tasks if v not in done and set(tasks[v]['deps'])<=done)
   feasible=[]
   for bits in itertools.product([0,1],repeat=len(ready)):
    chosen=[v for v,b in zip(ready,bits) if b];groups=[tasks[v]['group'] for v in chosen if tasks[v]['group'] is not None]
    if len(chosen)<=capacity and len(groups)==len(set(groups)):feasible.append((bits,chosen))
   batch=max(feasible)[1];assert batch
   expected.append(batch);done.update(batch)
  assert execution_batches(tasks,capacity)==expected,(case,'batches',capacity)
 def path_weights(v):
  ds=set(tasks[v]['deps']);weight=tasks[v]['duration']
  return [weight] if not ds else [weight+p for d in ds for p in path_weights(d)]
 expected=max([p for v in tasks for p in path_weights(v)],default=0)
 assert critical_path_length(tasks)==expected,(case,'critical_path')
 assert tasks==before,(case,'mutation')
print(json.dumps({'pass':True,'random_dags':1000,'max_nodes':6,'batch_capacities':[1,2,3,7],'seed':202609091,'oracles':['enumerate all valid topological permutations','enumerate feasible ready subsets','enumerate dependency paths']}))
