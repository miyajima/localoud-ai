"""Prepare once, no model/network calls. Never overwrite an existing run."""
import hashlib
import json
from pathlib import Path
import sys


def prepare(root):
    root.mkdir(parents=True, exist_ok=False)
    initial = {
        "slug.py": "def slugify(text):\n    raise NotImplementedError\n",
        "AGENTS.md": "Work only in this condition's fixture directory. Modify only slug.py. No network, delegation, commits or access to other condition directories.\n",
    }
    acceptance = '''import importlib.util
import pathlib
import sys
spec = importlib.util.spec_from_file_location("candidate", pathlib.Path(sys.argv[1]) / "slug.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
cases = [("Hello, World!", "hello-world"), (" A___B ", "a-b"), ("", ""), ("日本ABC", "abc"), ("a--b", "a-b"), ("123", "123")]
for value, expected in cases:
    assert module.slugify(value) == expected, (value, expected)
print("6 fixed cases passed")
'''
    texts = [
        ("purpose", "Implement slugify(text) in slug.py for this task only."),
        ("constraints", "Modify only slug.py. No network, delegation, commits or other folders. Use Python standard library only."),
        ("decisions", "Earlier proposal: retain Unicode letters."),
        ("current_state", "slug.py currently raises NotImplementedError; no implementation or verification has run."),
        ("unresolved", "No unresolved specification questions remain after the correction below."),
        ("completion", "Run the fixed acceptance.py against the condition fixture and inspect the diff. All six cases and the file scope must pass."),
        ("decisions", "Correction: use ASCII a-z and 0-9 only. Lowercase ASCII capitals, replace each run of all other characters with one hyphen, strip edge hyphens. Empty input returns empty string. Unicode letters must not be retained."),
        ("background", "Unrelated earlier conversation about presentation colors. " * 300),
    ]
    sources, groups = [], []
    for i, (section, text) in enumerate(texts):
        sources.append(dict(id=f"s{i}", role="user", reference=f"fixture-turn:{i}", text=text, call_id=None))
        groups.append(dict(id=f"g{i}", section=section, source_ids=[f"s{i}"], depends_on=[], corrects=["g2"] if i == 6 else []))
    plan = dict(title="Fixed handoff calibration", steps=[dict(key="slug", title="Slug", goal=texts[0][1] + " Follow the corrected ASCII policy.", dependencies=[], brief=dict(
        acceptance_criteria=[texts[5][1]], constraints=[texts[1][1]], context_items=[],
        budget=dict(initial_tokens=8000, max_total_tokens=10000, max_single_retrieval_tokens=500),
        handoff=dict(sources=sources, groups=groups, outcomes=[], max_bytes=6000)))])
    (root / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=2) + "\n")
    (root / "acceptance.py").write_text(acceptance)
    hashes = {name: hashlib.sha256(text.encode()).hexdigest() for name, text in initial.items()}
    for condition in ("all", "one", "extracted-none"):
        fixture = root / condition / "fixture"
        fixture.mkdir(parents=True)
        (root / condition / "evidence").mkdir()
        for name, text in initial.items():
            (fixture / name).write_text(text)
        assert {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in fixture.iterdir()} == hashes
    (root / "freeze.json").write_text(json.dumps(dict(schema="handoff-comparison-freeze/v1", initial_files=hashes,
        plan_sha256=hashlib.sha256((root / "plan.json").read_bytes()).hexdigest(),
        acceptance_sha256=hashlib.sha256(acceptance.encode()).hexdigest(),
        conditions={"all": "all", "one": "1", "extracted-none": "none"},
        status="prepared_not_executed", model=None, reasoning=None), indent=2) + "\n")


if __name__ == "__main__":
    prepare(Path(sys.argv[1]).resolve())
