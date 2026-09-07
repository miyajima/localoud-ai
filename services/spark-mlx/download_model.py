"""Download the pinned 8-bit checkpoint; never execute model-repository code."""
from pathlib import Path
import hashlib
import json
from huggingface_hub import snapshot_download

HERE = Path(__file__).resolve().parent
MODEL_ID = "abenzerps/Spark-X2.5-4B-MLX-8bit"
REVISION = (HERE / "model-revision.txt").read_text().strip()
DEST = HERE.parent.parent / "models" / "Spark-X2.5-4B-MLX-8bit"

def verify(path: Path):
    config = json.loads((path / "config.json").read_text())
    if config.get("model_type") != "spark2_5" or config.get("quantization", {}).get("bits") != 8:
        raise ValueError("The Hub requires the Spark2.5 8-bit checkpoint")
    index = json.loads((path / "model.safetensors.index.json").read_text())
    for filename in set(index["weight_map"].values()):
        if Path(filename).name != filename or not (path / filename).is_file():
            raise ValueError("Missing or invalid checkpoint shard")
    sums = path / "SHA256SUMS.txt"
    verified = []
    for line in sums.read_text().splitlines():
        digest, filename = line.split(maxsplit=1)
        filename = filename.strip().lstrip("*")
        if Path(filename).name != filename:
            raise ValueError("Unsafe checksum filename")
        f = path / filename
        if not f.exists():
            continue  # Remote Python code and artwork are deliberately not downloaded.
        h = hashlib.sha256()
        with f.open("rb") as stream:
            for chunk in iter(lambda: stream.read(8 * 1024 * 1024), b""):
                h.update(chunk)
        if h.hexdigest() != digest:
            raise ValueError(f"Checksum mismatch: {filename}")
        verified.append(filename)
    if "model.safetensors" not in verified:
        raise ValueError("Checkpoint checksum was not verified")
    return {"model": MODEL_ID, "revision": REVISION, "quantization_bits": 8, "verified_files": verified}

if __name__ == "__main__":
    snapshot_download(MODEL_ID, revision=REVISION, local_dir=DEST,
        allow_patterns=["*.json", "*.jinja", "model*.safetensors", "SHA256SUMS.txt"], max_workers=4)
    result = verify(DEST)
    (DEST / "hub-verification.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
