import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent
report = {'scope':'one_task_one_pair_code_generation_only','conditions':{}}
for stage in ('local','baseline','hybrid'):
    d = root/stage
    frozen = json.loads((d/'frozen-inputs.json').read_text())
    assert all(hashlib.sha256((root/name).read_bytes()).hexdigest()==digest for name,digest in frozen.items())
    metric = json.loads((d/'metrics.json').read_text())
    if stage != 'local':
        events = [json.loads(line) for line in (d/'events.jsonl').read_text().splitlines()]
        assert not any(e.get('item',{}).get('type') in ('command_execution','mcp_tool_call','file_change','web_search') for e in events)
        assert len(metric['usage_events']) == 1
        usage = metric['usage_events'][0]
        total = usage['input_tokens'] + usage['output_tokens']
    else:
        usage = metric['usage']
        total = usage['total_tokens']
    report['conditions'][stage] = {'usage':usage,'input_plus_output':total,'elapsed_seconds':metric['elapsed_seconds'],'acceptance_pass':(d/'tests.txt').read_text().rstrip().endswith('OK'),'source_sha256':hashlib.sha256((d/'intervals.py').read_bytes()).hexdigest()}
a = report['conditions']['baseline']['input_plus_output']
b = report['conditions']['hybrid']['input_plus_output']
report['cloud_token_difference'] = b-a
report['cloud_token_increase_percent'] = 100*(b/a-1)
report['hybrid_generation_seconds_sum'] = sum(report['conditions'][s]['elapsed_seconds'] for s in ('local','hybrid'))
(root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
