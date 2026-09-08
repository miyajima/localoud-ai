"""Deterministic app-server fixture: no model, network, or repository writes."""
import json, sys
thread = {"id": "fixture-thread", "turns": []}
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
    elif method in ("thread/read", "thread/resume"): result = {"thread": thread,"model":p.get("model",thread.get("model","fixture-a"))}
    elif method == "turn/start":
        if p.get("input", [{}])[0].get("text") == "verify reasoning":
            assert p.get("model") == "fixture-b", "wrong model sent"
            assert p.get("effort") == "high", "reasoning was not sent"
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
