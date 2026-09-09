"""No model or network. Capture the real worker prompt and complete one turn."""
import json
import pathlib
import sys

thread_start = None
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request.get("method")
    if method == "initialize":
        result = {"userAgent": "handoff-fixture"}
    elif method == "model/list":
        result = {"data": [{"id": "fixture-b", "model": "fixture-b", "supportedReasoningEfforts": [{"reasoningEffort": "high"}]}], "nextCursor": None}
    elif method == "thread/start":
        thread_start = request["params"]
        result = {"thread": {"id": "handoff-thread", "turns": []}, "model": thread_start.get("model", "fixture")}
    elif method == "turn/start":
        params = request["params"]
        if thread_start.get("model"):
            assert params.get("model") == thread_start["model"] == "fixture-b"
            assert params.get("effort") == "high"
        params["fixture_thread_start"] = thread_start
        pathlib.Path(sys.argv[1]).write_text(json.dumps(params, ensure_ascii=False))
        result = {"turn": {"id": "handoff-turn", "status": "inProgress", "items": []}}
    else:
        raise RuntimeError("unexpected method: " + str(method))
    print(json.dumps({"id": request["id"], "result": result}), flush=True)
    if method == "turn/start":
        print(json.dumps({"method": "turn/completed", "params": {
            "threadId": "handoff-thread", "turn": {"id": "handoff-turn", "status": "completed", "items": []}
        }}), flush=True)
