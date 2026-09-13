#!/usr/bin/env python3
"""Read-only export: payload measurements plus per-attempt provider usage/outcomes."""
import argparse
import json
import pathlib
import sqlite3

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--db", type=pathlib.Path, default=pathlib.Path.home() / "Library/Application Support/dev.locloud.astra-hub/hub.db")
parser.add_argument("--workflow", required=True)
args = parser.parse_args()
connection = sqlite3.connect(args.db.resolve().as_uri() + "?mode=ro", uri=True)
row = connection.execute("SELECT value FROM settings WHERE key=?", ("autonomous:" + args.workflow,)).fetchone()
if row is None:
    parser.error("workflow not found")
workflow = json.loads(row[0])
attempts = {item["iteration"]: item for item in workflow.get("history", [])}
attempts[workflow["iteration"]] = workflow
rows = connection.execute("SELECT key,value FROM settings WHERE key GLOB ? ORDER BY key", ("context_metrics:" + args.workflow + ":*",))
result = []
for key, value in rows:
    metric = json.loads(value)
    attempt = attempts.get(metric["iteration"], {})
    step = next((s for s in attempt.get("steps", []) if s["step"]["key"] == metric["step_key"]), None)
    metric["workflow_id"] = args.workflow
    metric["attempt_status"] = step.get("status") if step else None
    metric["model_target"] = step.get("target") if step else None
    metric["provider_usage_snapshots"] = step.get("usage") if step else None
    metric["review"] = attempt.get("review")
    result.append(metric)
print(json.dumps({"records": result, "notes": [
    "Read-only; no model calls. Empty records mean no measurements, not zero usage.",
    "Provider snapshots may be cumulative: do not sum them.",
    "Compare matched tasks, models and acceptance outcomes. Payload reduction alone does not prove end-to-end token, latency or quality gains.",
    "Additional reads are included in provider turn usage when reported, but are not separately attributed here."
]}, ensure_ascii=False, indent=2))
