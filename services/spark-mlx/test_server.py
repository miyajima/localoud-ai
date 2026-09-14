from fastapi.testclient import TestClient
import pytest
from server import create_app, parse_output, completion_output

SCHEMA = {"type":"object","properties":{"executor":{"enum":["codex"]}},"required":["executor"],"additionalProperties":False}
class Fake:
    info = {"model":"fixture","quantization_bits":8,"device":"fixture"}
    def generate(self, req):
        if req.task == 'input_completion':
            return '{"suffix":"を確認してください"}', {"prompt_tokens":10,"completion_tokens":5}
        return '{"executor":"codex"}', {"prompt_tokens":10,"completion_tokens":5}

def test_health_schema_and_browser_boundary():
    with TestClient(create_app(Fake)) as c:
        assert c.get('/health').json()['quantization_bits']==8
        req={"task":"route","messages":[{"role":"user","content":"fix"}],"schema":SCHEMA}
        r=c.post('/v1/generate',json=req)
        assert r.status_code==200 and r.json()['output']=={"executor":"codex"}
        assert c.post('/v1/generate',json=req,headers={"origin":"https://hostile.example"}).status_code==403
        req['schema']={"$ref":"https://hostile.example/schema"}
        assert c.post('/v1/generate',json=req).status_code==422

def test_completion_preserves_prefix_and_returns_only_suffix():
    import json
    assert json.loads(completion_output('Fix the typo and check it.', 'Fix the typo')) == {'suffix':' and check it.'}
    assert json.loads(completion_output('Fix the typo', 'Fix the typo')) == {'suffix':''}
    with pytest.raises(ValueError): completion_output('Here is the answer', 'Fix the typo')


def test_input_completion_capability_and_suffix():
    with TestClient(create_app(Fake)) as c:
        assert c.get('/health').json()['capabilities']['input_completion'] is True
        schema={"type":"object","properties":{"suffix":{"type":"string","maxLength":320}},"required":["suffix"],"additionalProperties":False}
        result=c.post('/v1/generate',json={"task":"input_completion","model":"fixture","messages":[{"role":"user","content":"入力補完"}],"schema":schema,"max_tokens":400})
        assert result.status_code == 200
        assert result.json()['output'] == {"suffix":"を確認してください"}


def test_malformed_or_extra_output_is_not_accepted():
    for text in ['thinking {"executor":"codex"}', '{"executor":"codex","extra":true}', '{"executor":"spark"}']:
        with pytest.raises(Exception):parse_output(text,SCHEMA)


def test_model_selection_is_checked_before_generation():
    with TestClient(create_app(Fake)) as c:
        req={"task":"route","model":"fixture","messages":[{"role":"user","content":"fix"}],"schema":SCHEMA}
        r=c.post('/v1/generate',json=req)
        assert r.status_code==200 and r.json()['model']=='fixture'
        req['model']='another-model'
        assert c.post('/v1/generate',json=req).status_code==409


def test_draft_handoff_is_an_explicit_local_task():
    from server import GenerationRequest
    request = GenerationRequest(
        task='draft_handoff',
        messages=[{'role':'user','content':'visible conversation'}],
        schema={'type':'object'},
        max_tokens=4096,
    )
    assert request.task == 'draft_handoff'


def test_replacement_config_checks_real_checkpoint_bits_and_shards(tmp_path, monkeypatch):
    from model_config import ModelConfig
    import json
    checkpoint = tmp_path / 'weights'
    checkpoint.mkdir()
    (checkpoint / 'config.json').write_text(json.dumps({'quantization': {'bits': 4}}))
    spec = tmp_path / 'model.json'
    spec.write_text(json.dumps({'model_id':'fixture/next','path':'weights','loader':'mlx_lm','quantization_bits':4}))
    monkeypatch.setenv('LOCAL_MODEL_CONFIG', str(spec))
    config = ModelConfig.from_environment()
    with pytest.raises(ValueError): config.verify()
    (checkpoint / 'model.safetensors').write_bytes(b'fixture')
    assert config.verify()['model']=='fixture/next'
    (checkpoint / 'config.json').write_text(json.dumps({'quantization': {'bits': 8}}))
    with pytest.raises(ValueError): config.verify()

def test_completion_preserves_leading_newlines_and_whitespace():
    import json
    prefix = '\n  READMEの誤字を修正して'
    assert json.loads(completion_output(prefix + 'ください。', prefix)) == {'suffix': 'ください。'}

def test_backend_prefills_exact_prefix_and_validates_only_generated_tail(monkeypatch):
    import sys
    import json
    from types import SimpleNamespace
    from server import MLXBackend, GenerationRequest
    captured = {}
    class Tokenizer:
        def apply_chat_template(self, messages, **kwargs):
            captured.update(messages=messages, kwargs=kwargs)
            return 'fixture prompt'
        def encode(self, prompt): return [1]
    monkeypatch.setitem(sys.modules, 'mlx_lm', SimpleNamespace(stream_generate=lambda *a, **k: iter([SimpleNamespace(text='ください。', prompt_tokens=12, generation_tokens=3)])))
    monkeypatch.setitem(sys.modules, 'mlx_lm.sample_utils', SimpleNamespace(make_sampler=lambda **k: None))
    backend = object.__new__(MLXBackend)
    backend.model, backend.tokenizer = None, Tokenizer()
    prefix = '\nREADMEの誤字を修正して'
    req = GenerationRequest(task='input_completion', messages=[{'role':'user','content':json.dumps({'prefix':prefix,'past_inputs':[]})}], schema={'type':'object'})
    text, usage = backend.generate(req)
    assert captured['messages'][-1] == {'role':'assistant','content':prefix}
    assert captured['kwargs']['continue_final_message'] is True
    assert captured['kwargs']['add_generation_prompt'] is False
    assert json.loads(text) == {'suffix':'ください。'}
    assert usage == {'prompt_tokens':12,'completion_tokens':3}

def test_finished_instruction_does_not_run_inference(monkeypatch):
    import sys
    import json
    from types import SimpleNamespace
    from server import MLXBackend, GenerationRequest
    def unexpected(*a, **k): raise AssertionError('complete sentence must not invoke the model')
    monkeypatch.setitem(sys.modules, 'mlx_lm', SimpleNamespace(stream_generate=unexpected))
    monkeypatch.setitem(sys.modules, 'mlx_lm.sample_utils', SimpleNamespace(make_sampler=unexpected))
    backend = object.__new__(MLXBackend)
    for prefix in ['READMEの誤字を修正して、内容を確認してください。', 'Fix the typo.']:
        req = GenerationRequest(task='input_completion', messages=[{'role':'user','content':json.dumps({'prefix':prefix})}], schema={'type':'object'})
        text, usage = backend.generate(req)
        assert json.loads(text) == {'suffix':''}
        assert usage == {'prompt_tokens':0,'completion_tokens':0}
