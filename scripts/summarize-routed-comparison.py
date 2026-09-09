"""Read-only session accounting for the bounded Astra/Luna route calibration.

Only token_usage_record is summed (not cumulative token_count snapshots). Raw
conversation/tool text is never copied into the report. Missing billing remains
unknown; this script cannot prove account-wide isolation or quota savings.
"""
import datetime as dt
import json
from pathlib import Path
import sys


def timestamp(value):
    return dt.datetime.fromisoformat(value.replace(" UTC", "+00:00").replace("Z", "+00:00"))


def session(path):
    meta, contexts, records, rates = {}, {}, {}, []
    for line in path.open():
        row = json.loads(line)
        payload = row.get("payload", {})
        if row.get("type") == "session_meta":
            meta = payload
        elif row.get("type") == "turn_context":
            contexts[payload.get("turn_id")] = {
                "model": payload.get("model"), "reasoning": payload.get("effort")}
        elif row.get("type") == "token_usage_record" and payload.get("thread_id") == meta.get("id"):
            key = payload.get("response_id")
            if not key:
                raise ValueError("usage record lacks response ID")
            if key in records and records[key]["usage"] != payload["usage"]:
                raise ValueError("conflicting duplicate response usage")
            records[key] = {"timestamp": row["timestamp"], **payload}
        elif payload.get("type") == "token_count" and payload.get("rate_limits"):
            rates.append({"timestamp": row["timestamp"], "rate_limits": payload["rate_limits"]})
    return {"path": str(path), "meta": meta, "contexts": contexts,
            "records": list(records.values()), "rates": rates}


def aggregate(records):
    keys = ["input_tokens", "cached_input_tokens", "output_tokens", "reasoning_output_tokens", "total_tokens"]
    totals = {key: sum(row["usage"][key] for row in records) for key in keys}
    totals["non_cached_input_tokens"] = totals["input_tokens"] - totals["cached_input_tokens"]
    assert totals["non_cached_input_tokens"] >= 0
    assert totals["total_tokens"] == totals["input_tokens"] + totals["output_tokens"]
    return {"model_responses": len(records), **totals}


def summarize(root, paths):
    streams = {name: session(path) for name, path in paths.items()}
    latest_parent_turn = next(reversed(streams["parent"]["contexts"]))
    parent = [r for r in streams["parent"]["records"] if r["turn_id"] == latest_parent_turn]
    own_roots = {latest_parent_turn, *streams["localoud"]["contexts"].keys(), *streams["native"]["contexts"].keys()}
    phases = {}
    for name in ("native", "localoud"):
        evidence = root / name / "evidence"
        before = json.loads((evidence / "quota-before.json").read_text())
        after = json.loads((evidence / "quota-after.json").read_text())
        begin, end = timestamp(before["timestamp"]), timestamp(after["timestamp"])
        parent_records = [r for r in parent if begin <= timestamp(r["timestamp"]) <= end]
        children = streams[name]["records"]
        # Independent per-response records must reconcile with the final child total.
        child_sum = aggregate(children)
        assert child_sum["total_tokens"] == children[-1]["thread_token_usage"]["total_tokens"]
        review = json.loads((evidence / "review-final.json").read_text())
        phases[name] = {"begin": before["timestamp"], "end": after["timestamp"],
            "elapsed_seconds_through_parent_review": (end-begin).total_seconds(),
            "parent": aggregate(parent_records), "implementation_child": child_sum,
            "parent_plus_child": aggregate(parent_records + children),
            "models_verified_from_turn_context": streams[name]["contexts"],
            "child_thread_id": streams[name]["meta"]["id"], "source": streams[name]["meta"].get("source"),
            "cli_version": streams[name]["meta"].get("cli_version"),
            "repair_turns": review["repair_turns"], "quality": review["verdict"],
            "quota_before": before["rateLimitsByLimitId"], "quota_after": after["rateLimitsByLimitId"],
            "displayed_used_percent_delta": after["rateLimitsByLimitId"]["codex"]["primary"]["usedPercent"] - before["rateLimitsByLimitId"]["codex"]["primary"]["usedPercent"],
            "attributable_quota_consumption": None, "unrelated_records": [], "platform_records": []}
    # Positive contamination evidence only; this date directory is not a whole-account audit.
    for path in paths["parent"].parent.glob("*.jsonl"):
        if path in paths.values() or path.stat().st_mtime < timestamp(phases["native"]["begin"]).timestamp():
            continue
        stream = session(path)
        for record in stream["records"]:
            for phase in phases.values():
                if timestamp(phase["begin"]) <= timestamp(record["timestamp"]) <= timestamp(phase["end"]):
                    field = "platform_records" if record.get("root_turn_id") in own_roots else "unrelated_records"
                    phase[field].append(record)
    for phase in phases.values():
        other = phase.pop("unrelated_records")
        phase["concurrent_unrelated_usage_record_count"] = len(other)
        phase["concurrent_unrelated_thread_count"] = len({r["thread_id"] for r in other})
        platform = phase.pop("platform_records")
        phase["observed_related_platform_usage"] = aggregate(platform)
        phase["platform_quota_accounting"] = "unknown; observed usage is not proof of subscription charge"
    native_begin = timestamp(phases["native"]["begin"])
    localoud_end = timestamp(phases["localoud"]["end"])
    preparation = [r for r in parent if timestamp(r["timestamp"]) < native_begin]
    post_run = [r for r in parent if timestamp(r["timestamp"]) > localoud_end]
    between = [r for r in parent if timestamp(phases["native"]["end"]) < timestamp(r["timestamp"]) < timestamp(phases["localoud"]["begin"])]
    assert len(preparation) + len(post_run) + len(between) + sum(p["parent"]["model_responses"] for p in phases.values()) == len(parent)
    report = {"schema": "astra-luna-route-comparison/v1", "status": "inconclusive", "runs": phases,
        "parent_model": streams["parent"]["contexts"][latest_parent_turn],
        "parent_thread_id": streams["parent"]["meta"]["id"], "parent_turn_id": latest_parent_turn,
        "parent_setup_and_common_plan": aggregate(preparation),
        "inter_condition_parent_overhead": aggregate(between),
        "post_run_analysis_parent_checkpoint": aggregate(post_run),
        "full_parent_turn_checkpoint": aggregate(parent),
        "checkpoint_timestamp": parent[-1]["timestamp"],
        "accounting": "Each response ID counted once, matching owning thread ID. Reasoning tokens are included in output, not added again. Parent phase windows include extraction/dispatch, waiting, verification/review and repair. Setup/common planning is separate and shared, not claimed free; cannot separate implementation engineering from plan preparation in this session. Post-run analysis is retained separately through this checkpoint; final response usage is not yet known.",
        "quota_savings": None,
        "limits": ["Single small synthetic case, sequential order not randomized", "Native reads full brief from a file with fork_turns=1; Localoud receives extracted capsule in an independent worker; route and context delivery both differ", "Original Astra parent retains a large live conversation; this is not an isolated parent benchmark", "Same fixed six cases plus two review-derived counterexamples; one retained native repair, no restart", "Common planning/setup cannot be billed to one condition precisely", "Other task usage observed; account/device isolation not established", "Used-percent values are integer in observed snapshots; precision, rounding and update lag unknown", "Platform checks and post-run reporting have separately recorded overhead; billable attribution unavailable", "Total or non-cached token differences are not subscription utilization savings"],
        "session_sources": {k: {"path": v["path"], "id": v["meta"]["id"]} for k, v in streams.items()}}
    return report


if __name__ == "__main__":
    root = Path(sys.argv[1])
    paths = dict(zip(("parent", "native", "localoud"), map(Path, sys.argv[2:5])))
    report = summarize(root, paths)
    (root / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({name: {k: v[k] for k in ("parent", "implementation_child", "parent_plus_child", "repair_turns", "quality", "concurrent_unrelated_thread_count")} for name, v in report["runs"].items()}, ensure_ascii=False))
