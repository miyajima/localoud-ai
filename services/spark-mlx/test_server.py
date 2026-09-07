from fastapi.testclient import TestClient
import pytest
from server import create_app, parse_output

SCHEMA = {"type":"object","properties":{"executor":{"enum":["codex"]}},"required":["executor"],"additionalProperties":False}
class Fake:
    info = {"model":"fixture","quantization_bits":8,"device":"fixture"}
    def generate(self, req):
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

def test_malformed_or_extra_output_is_not_accepted():
    for text in ['thinking {"executor":"codex"}', '{"executor":"codex","extra":true}', '{"executor":"spark"}']:
        with pytest.raises(Exception):parse_output(text,SCHEMA)
