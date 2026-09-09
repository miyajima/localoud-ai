from pathlib import Path
import json,hashlib,shutil,re,datetime,subprocess
r=Path(__file__).resolve().parent
repo=Path('/Users/miyajimakazuhiro/projects/localoud')
out=repo/'docs/evidence/luna-pipeline-benchmark-2026-09-09'
out.mkdir(exist_ok=False)
setup=json.loads((r/'setup.json').read_text())
planner=json.loads((r/'planner-usage.json').read_text())
session_dir=Path('/Users/miyajimakazuhiro/.codex/sessions/2026/09/09')
def collect(thread_id,model,effort):
 log=next(session_dir.glob(f'*{thread_id}.jsonl'))
 rows=[json.loads(l) for l in log.open()];meta=rows[0]['payload'];created=meta['timestamp']
 tokens=[];contexts=[];completed=[]
 for v in rows:
  if v.get('timestamp','')<created:continue
  p=v.get('payload',{})
  if v['type']=='event_msg' and p.get('type')=='token_count' and p.get('info'):
   row={'timestamp':v['timestamp'],**p['info']}
   if not tokens or row['total_token_usage']!=tokens[-1]['total_token_usage']:tokens.append(row)
  if v['type']=='turn_context':contexts.append({'model':p.get('model'),'effort':p.get('effort')})
  if v['type']=='event_msg' and p.get('type')=='task_complete':completed.append(v['timestamp'])
 assert tokens and contexts[-1]=={'model':model,'effort':effort},(thread_id,contexts)
 total=tokens[-1]['total_token_usage']
 for k in ['input_tokens','cached_input_tokens','output_tokens']:assert sum(t['last_token_usage'][k] for t in tokens)==total[k],(thread_id,k)
 def stamp(s):return datetime.datetime.fromisoformat(s.replace('Z','+00:00'))
 metrics={'thread_id':thread_id,'model':model,'effort':effort,'first_input_tokens':tokens[0]['last_token_usage']['input_tokens'],'input_tokens':total['input_tokens'],'cached_input_tokens':total['cached_input_tokens'],'uncached_input_tokens':total['input_tokens']-total['cached_input_tokens'],'output_tokens':total['output_tokens'],'total_tokens':total['input_tokens']+total['output_tokens'],'inference_count':len(tokens),'elapsed_seconds':(stamp(completed[-1])-stamp(created)).total_seconds() if completed else None}
 provenance={'session_id':meta['id'],'source':meta.get('source'),'created_at':created,'completed_at':completed[-1] if completed else None,'model':contexts[-1],'token_counts':tokens,'source_log_sha256':hashlib.sha256(log.read_bytes()).hexdigest()}
 return metrics,provenance
results={}
for lane in ['native','localoud']:
 d=out/lane;d.mkdir()
 impl_id='01a084f3-0f86-73c2-b9c6-62f1a6b760f8' if lane=='native' else json.loads((r/'localoud-start.json').read_text())['thread_id']
 impl,ip=collect(impl_id,'gpt-5.6-luna','max')
 if lane=='localoud':
  lr=json.loads((r/'localoud-result.json').read_text());assert lr['status']=='completed';impl['elapsed_seconds']=lr['elapsed_ms']/1000
 validation=json.loads((r/f'{lane}-validation.json').read_text())
 review_dir=r/f'{lane}-review';rv=json.loads((review_dir/'result.json').read_text());assert rv['status']=='completed'
 es=[json.loads(l) for l in (review_dir/'events.jsonl').open()]
 ids={e.get('thread_id') for e in es if e.get('thread_id')};assert len(ids)==1
 review,rp=collect(ids.pop(),'gpt-6-astra','low');review['elapsed_seconds']=rv['elapsed_ms']/1000
 request=json.loads((r/f'{lane}-review-request.json').read_text())
 for name,h in request['artifact_hashes'].items():
  path=(r/'oracle.py') if name=='oracle.py' else r/lane/name
  assert hashlib.sha256(path.read_bytes()).hexdigest()==h,(lane,name,'changed after review input')
 for n in ['scheduler.py','test_scheduler.py','acceptance.py','policy.md','PLAN.md','AGENTS.md']:shutil.copy2(r/lane/n,d/n)
 for name,data in [('implementation-usage.json',ip),('review-usage.json',rp),('review.json',rv),('validation.json',validation),('review-request.json',request)]:
  (d/name).write_text(json.dumps(data,indent=2)+'\n')
 passed=validation['tests']['exit_code']==0 and validation['oracle']['exit_code']==0 and all(validation['integrity'].values())
 quality={'fixed_tests_passed':12 if validation['tests']['exit_code']==0 else None,'own_tests_passed':int(re.search(r'Ran (\d+) tests',validation['tests']['stderr'])[1])-12 if validation['tests']['exit_code']==0 else None,'common_random_dags_passed':1000 if validation['oracle']['exit_code']==0 else None,'integrity_pass':all(validation['integrity'].values()),'review_verdict':rv['review']['verdict'],'review_findings':rv['review']['findings'],'validation_pass':passed}
 sums={k:impl[k]+review[k] for k in ['input_tokens','cached_input_tokens','uncached_input_tokens','output_tokens','total_tokens']}
 total={k:sums[k]+(planner['uncached_input_tokens'] if k=='uncached_input_tokens' else planner['usage'][k]) for k in sums}
 results[lane]={'implementation':impl,'review':review,'quality':quality,'implementation_plus_review':sums,'shared_plan_plus_implementation_plus_review':total,'baseline_commit':subprocess.check_output(['git','-C',str(r/lane),'rev-parse','HEAD'],text=True).strip()}
change={stage:{k:round((results['localoud'][stage][k]/results['native'][stage][k]-1)*100,3) for k in ['input_tokens','uncached_input_tokens','output_tokens','total_tokens']} for stage in ['implementation','implementation_plus_review','shared_plan_plus_implementation_plus_review']}
report={'date':'2026-09-09','scope':'Astra/low plan, Luna/max implementation, Astra/low review for each lane','setup':setup,'shared_planning':planner,'results':results,'localoud_change_percent':change,'limitations':['One task, one implementation per lane, one separate read-only review per lane; no best-of selection.','The frozen Astra/low plan and its measured authoring inference are reused from the previous experiment, not regenerated. Totals include that common measured plan once per hypothetical lane.','Native uses fork_turns=1 to permit the user-selected Luna/max override, unlike previous full-history native comparisons. Localoud uses independent thread/start.','Both reviewers use identical structured read-only app-server routing, rubric and neutral candidate cwd; candidate source differs. This does not compare native reviewer spawning with Localoud reviewer spawning.','Controller validation is supplied to reviewers; reviewers do not run tests themselves.','Provider instructions, tools, context and cache warmth are not identical.','Pipeline totals omit experiment preparation, parent orchestration/validation/reporting, platform guardians and unrelated session history costs. No subscription or full-session cost claims.','Previous runs used a different implementation model and/or native history settings, so cross-run differences are not a model-only causal comparison.']}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
for n in ['PLAN.md','policy.md','acceptance.py','oracle.py','task.txt','planner-usage.json','prepare_review.py','collect_pipeline.py']:shutil.copy2(r/n,out/n)
print(json.dumps({'results':results,'change':change},indent=2))
