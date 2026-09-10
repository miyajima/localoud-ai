"""Explicit single-attempt generation steps; never executes model-returned code."""
import hashlib
import json
import pathlib
import subprocess
import sys
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent
MODEL = '/Users/miyajimakazuhiro/.cache/huggingface/hub/models--abenzerps--Spark-X2.5-4B-MLX-8bit/snapshots/10d5396e7975f0647241da14d275113fa60116b3'
SCHEMA = {'type':'object','properties':{'code':{'type':'string'}},'required':['code'],'additionalProperties':False}

def write(path, value):
    path.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')

def main():
    stage = sys.argv[1]
    dest = ROOT / stage
    dest.mkdir(exist_ok=False)
    contract = (ROOT/'contract.txt').read_text()
    write(dest/'frozen-inputs.json',{name:hashlib.sha256((ROOT/name).read_bytes()).hexdigest() for name in ['contract.txt','acceptance.py','run.py']})
    prompt = contract
    if stage == 'hybrid':
        prompt += '\nInspect this draft implementation, correct all contract violations, and return the complete final module even if unchanged:\n'+(ROOT/'local'/'intervals.py').read_text()
    elif stage not in ('local','baseline'):
        previous = sys.argv[2]
        prompt += '\nCorrect this candidate according to the contract:\n'+(ROOT/previous/'intervals.py').read_text()
        prompt += '\nAcceptance feedback:\n'+(ROOT/previous/'tests.txt').read_text()
    (dest/'prompt.txt').write_text(prompt)
    started = time.monotonic()
    if stage == 'local':
        body = {'model':MODEL,'messages':[{'role':'system','content':'Return valid JSON only, with one code field containing the complete Python module. No markdown or thinking.'},{'role':'user','content':prompt}], 'max_tokens':4096,'temperature':0,'chat_template_kwargs':{'enable_thinking':False}}
        write(dest/'request.json',body)
        req = urllib.request.Request('http://127.0.0.1:18085/v1/chat/completions',data=json.dumps(body).encode(),headers={'Content-Type':'application/json'})
        with urllib.request.urlopen(req,timeout=300) as response:
            result = json.load(response)
        write(dest/'response.json',result)
        write(dest/'metrics.json',{'elapsed_seconds':time.monotonic()-started,'usage':result.get('usage'),'model':result.get('model'),'requested_model':MODEL})
        raw = result['choices'][0]['message']['content'].strip()
        if raw.startswith('```json\n') and raw.endswith('```'): raw = raw[8:-3].strip()
    else:
        schema = dest/'schema.json'
        write(schema,SCHEMA)
        work = pathlib.Path('/private/tmp/localoud-local-luna-20260910')/stage
        work.mkdir(parents=True,exist_ok=False)
        command = ['codex','exec','--ignore-user-config','--ephemeral','--skip-git-repo-check','-C',str(work),'-s','read-only','-m','gpt-5.6-luna','-c','model_reasoning_effort="max"','--json','--output-schema',str(schema),'-o',str(dest/'final.json'),'-']
        write(dest/'command.json',command)
        with (dest/'events.jsonl').open('w') as out, (dest/'stderr.txt').open('w') as err:
            p = subprocess.run(command,input=prompt,text=True,stdout=out,stderr=err,timeout=600)
        events = [json.loads(line) for line in (dest/'events.jsonl').read_text().splitlines() if line.strip()]
        usages = [e['usage'] for e in events if e.get('type')=='turn.completed']
        write(dest/'metrics.json',{'elapsed_seconds':time.monotonic()-started,'returncode':p.returncode,'usage_events':usages,'requested_model':'gpt-5.6-luna','reasoning':'max'})
        if p.returncode: raise RuntimeError('Codex failed; preserve logs and do not automatically retry')
        raw = (dest/'final.json').read_text()
    code = json.loads(raw)['code']
    if not isinstance(code,str): raise TypeError('code is not a string')
    (dest/'intervals.py').write_text(code)
    print(json.dumps({'stage':stage,'metrics':json.loads((dest/'metrics.json').read_text()),'source_bytes':len(code.encode())}))

if __name__ == '__main__': main()
