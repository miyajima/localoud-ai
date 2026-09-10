"""Extract the already-saved MiniCPM fenced code response; no model call."""
import json
from pathlib import Path

root = Path(__file__).resolve().parent
dest = root / "minicpm-f16"
response = json.loads((dest / "response.json").read_text())
content = response["choices"][0]["message"]["content"].strip()
if content.startswith("```") and content.endswith("```"):
    first_newline = content.find("\n")
    content = content[first_newline + 1:-3].strip()
if not content:
    raise ValueError("saved response contains no code")
(dest / "intervals.py").write_text(content + "\n")
print(f"extracted_bytes={len((content + chr(10)).encode())}")
