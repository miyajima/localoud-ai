"""One local-only repetition. Preserve the original Spark request verbatim except model."""
import hashlib
import json
from pathlib import Path
import time
import urllib.request

root = Path(__file__).resolve().parent
dest = root/'qwen-local'
dest.mkdir(exist_ok=False)
body = json.loads((root/'local/request.json').read_text())
body['model'] = 'mtplx-qwen38-27b-optimized-speed'
assert body['messages'][1]['content'] == (root/'contract.txt').read_text()
(dest/'request.json').write_text(json.dumps(body,ensure_ascii=False,indent=2)+'\n')
(dest/'frozen-inputs.json').write_text(json.dumps({n:hashlib.sha256((root/n).read_bytes()).hexdigest() for n in ['contract.txt','acceptance.py','local/request.json','run_qwen.py']},indent=2)+'\n')
started = time.monotonic()
request = urllib.request.Request('http://127.0.0.1:18086/v1/chat/completions',data=json.dumps(body).encode(),headers={'Content-Type':'application/json'})
with urllib.request.urlopen(request,timeout=300) as response:
    result = json.load(response)
elapsed = time.monotonic()-started
(dest/'response.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
metrics = {'elapsed_seconds':elapsed,'usage':result.get('usage'),'requested_model':body['model'],'response_model':result.get('model'),'finish_reason':result['choices'][0].get('finish_reason')}
(dest/'metrics.json').write_text(json.dumps(metrics,indent=2)+'\n')
raw = result['choices'][0]['message']['content'].strip()
if raw.startswith('```json\n') and raw.endswith('```'): raw = raw[8:-3].strip()
code = json.loads(raw)['code']
assert isinstance(code,str)
(dest/'intervals.py').write_text(code)
print(json.dumps(metrics))
