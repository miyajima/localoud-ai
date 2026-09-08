"""Wire acceptance fixture: no model calls, files or network access."""
import json, sys
thread = {'id':'composer-thread','turns':[]}
goal = None
def emit(value): print(json.dumps(value),flush=True)
for line in sys.stdin:
    v=json.loads(line)
    if v.get('id')=='question-1' and 'method' not in v:
        assert v['result']['answers']=={'scope':{'answers':['Small']}}
        thread['turns'][0]['status']='completed'
        emit({'method':'item/completed','params':{'threadId':thread['id'],'turnId':'plan-turn','item':{'type':'agentMessage','id':'answer','text':'Answer accepted'}}})
        emit({'method':'turn/completed','params':{'threadId':thread['id'],'turn':thread['turns'][0]}})
        continue
    if 'method' not in v or 'id' not in v: continue
    method=v['method'];p=v.get('params',{})
    if method=='initialize':result={}
    elif method=='model/list':result={'data':[{'id':'fixture-model','model':'fixture-model','supportedReasoningEfforts':[{'reasoningEffort':'high'}]}],'nextCursor':None}
    elif method=='collaborationMode/list':result={'data':[{'mode':'plan'},{'mode':'default'}]}
    elif method=='skills/list':result={'data':[{'cwd':p['cwds'][0],'errors':[],'skills':[{'enabled':True,'name':'fixture-skill','path':'/fixture/SKILL.md','description':'test skill'}]}]}
    elif method=='plugin/installed':result={'marketplaces':[{'name':'fixture-market','plugins':[{'id':'fixture-plugin@fixture-market','name':'fixture-plugin','enabled':True,'installed':True}]}]}
    elif method=='thread/start':result={'thread':thread,'model':p['model']}
    elif method in ['thread/read','thread/resume']:result={'thread':thread,'model':'fixture-model'}
    elif method=='thread/goal/set':
        assert 'tokenBudget' not in p
        if 'objective' in p:goal={'objective':p['objective'],'status':p['status']}
        else:goal['status']=p['status']
        result={'goal':goal}
    elif method=='thread/goal/get':result={'goal':goal}
    elif method=='turn/start':
        assert p['model']=='fixture-model' and p['effort']=='high'
        c=p['collaborationMode'];assert c['settings']=={'model':'fixture-model','reasoning_effort':'high','developer_instructions':None}
        if c['mode']=='plan':
            assert len(p['input'])==3
            assert p['input'][1]=={'type':'skill','name':'fixture-skill','path':'/fixture/SKILL.md'}
            assert p['input'][2]=={'type':'mention','name':'fixture-plugin','path':'plugin://fixture-plugin@fixture-market'}
            text=p['input'][0]['text'];assert '$fixture-skill' in text and '@fixture-plugin' in text and 'explorer' in text and '/fixture/readme.md' in text
            turn={'id':'plan-turn','status':'inProgress','items':[]}
        else:
            assert c['mode']=='default' and goal['objective']=='Finish fixture'
            turn={'id':'goal-turn','status':'inProgress','items':[]}
        thread['turns']=[turn];result={'turn':turn}
    elif method=='turn/interrupt':
        thread['turns'][0]['status']='interrupted';result={}
    else:
        emit({'id':v['id'],'error':{'message':'unexpected '+method}});continue
    emit({'id':v['id'],'result':result})
    if method=='turn/start' and p['collaborationMode']['mode']=='plan':
        emit({'id':'question-1','method':'item/tool/requestUserInput','params':{'threadId':thread['id'],'turnId':'plan-turn','itemId':'input','isBlocking':True,'questions':[{'id':'scope','header':'Scope','question':'Which scope?','options':[{'label':'Small','description':'Bounded change'},{'label':'Large','description':'Broad change'}]}]}})
