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
