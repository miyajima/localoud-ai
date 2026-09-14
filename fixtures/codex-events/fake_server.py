"""Deterministic app-server fixture: no model, network, or repository writes."""
import json, sys
thread = {"id": "fixture-thread", "turns": []}
archived = False
for line in sys.stdin:
    v = json.loads(line)
    method = v.get("method")
    if "id" not in v:
        continue
    p = v.get("params", {})
    if method == "initialize": result = {"userAgent": "fixture"}
    elif method == "model/list": result = {"data":[{"id":"fixture-a","model":"fixture-a","isDefault":True,"supportedReasoningEfforts":[{"reasoningEffort":"low"}]},{"id":"fixture-b","model":"fixture-b","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"high"}]}],"nextCursor":None}
    elif method == "thread/start":
        thread["model"] = p.get("model", "fixture-a")
        result = {"thread": thread,"model":thread["model"]}
    elif method == "thread/archive":
        if "--reject-archive" in sys.argv or any(t["status"] == "inProgress" for t in thread["turns"]):
            print(json.dumps({"id":v["id"],"error":{"message":"archive refused"}}),flush=True)
            continue
        assert p["threadId"] == thread["id"]
        archived = True
        result = {}
    elif method == "thread/unarchive":
        if "--reject-unarchive" in sys.argv:
            print(json.dumps({"id":v["id"],"error":{"message":"restore refused"}}),flush=True)
            continue
        assert p["threadId"] == thread["id"]
        archived = False
        result = {"thread":thread}
    elif method in ("thread/read", "thread/resume"):
        if archived and method == "thread/resume":
            print(json.dumps({"id":v["id"],"error":{"message":"archived thread must not be resumed"}}),flush=True)
            continue
        result = {"thread": thread,"model":p.get("model",thread.get("model","fixture-a"))}
    elif method == "turn/start":
        if p.get("input", [{}])[0].get("text") == "verify reasoning":
            assert p.get("model") == "fixture-b", "wrong model sent"
            assert p.get("effort") == "high", "reasoning was not sent"
        text = p.get("input", [{}])[0].get("text", "")
        if "<context_capsule_json>" in text:
            assert "application/vnd.localoud.context-capsule+json" in text
            assert "<prior_transcript_json>" not in text
        if text.startswith("verify isolated cwd:"):
            expected = text.removeprefix("verify isolated cwd:")
            assert p.get("cwd") == expected, "turn reused a stale working directory"
            policy = p.get("sandboxPolicy", {})
            assert policy.get("type") == "workspaceWrite"
            assert policy.get("writableRoots") == [expected]
            assert policy.get("excludeSlashTmp") and policy.get("excludeTmpdirEnvVar")
            assert p.get("approvalPolicy") == "on-request"
        turn = {"id": "fixture-turn", "status": "inProgress", "items": []}
        thread["turns"] = [turn]
        result = {"turn": turn}
    elif method == "turn/steer":
        assert p["expectedTurnId"] == "fixture-turn"
        result = {"turnId": "fixture-turn"}
    elif method == "turn/interrupt":
        thread["turns"][0]["status"] = "interrupted"
        result = {}
    else:
        print(json.dumps({"id": v["id"], "error": {"message": "unexpected method"}}), flush=True)
        continue
    print(json.dumps({"id": v["id"], "result": result}), flush=True)
    if method == "turn/start":
        print(json.dumps({"method":"item/agentMessage/delta", "params":{"threadId":"fixture-thread","turnId":"fixture-turn","itemId":"msg","delta":"fixture output"}}),flush=True)
    if method == "turn/interrupt":
        print(json.dumps({"method":"turn/completed", "params":{"threadId":"fixture-thread","turn":thread["turns"][0]}}),flush=True)
