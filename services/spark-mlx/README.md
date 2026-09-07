# Spark MLX 8-bit service

Uses the community MLX 8-bit conversion `abenzerps/Spark-X2.5-4B-MLX-8bit` at the pinned revision in `model-revision.txt`, with XHToken's architecture adapter pinned in `requirements.txt`. No downloaded model Python is executed. Model weight SHA-256 is checked before loading. The package's strict tensor loading and an actual generation test are separate checks.

```sh
uv venv services/spark-mlx/.venv --python 3.12
uv pip install --python services/spark-mlx/.venv/bin/python -r services/spark-mlx/requirements.txt
services/spark-mlx/.venv/bin/python services/spark-mlx/download_model.py
services/spark-mlx/.venv/bin/python services/spark-mlx/server.py
```

On macOS, launch the service from a session where Metal is visible. It binds only to `127.0.0.1:8765`. Override the port with `SPARK_PORT` and checkpoint directory with `SPARK_MODEL_PATH`. Host requests from browsers and non-loopback hosts are rejected.

- `GET /health`: actual loaded model, 8-bit precision, device, revision.
- `POST /v1/generate`: `task`, `messages`, JSON `schema`, `max_tokens`, `temperature`.
- Output JSON is parsed and validated. This is schema validation after generation, not a claim of grammar-constrained decoding. Malformed output is rejected with 422. Busy inference returns 429; the Rust provider serializes in-process calls to avoid overlapping MLX requests. Validation errors report a bounded schema diagnostic without exposing generated payloads.
- One dedicated inference thread owns MLX. Inputs are bounded at 8192 tokens; output at 4096.
- The service has no shell or filesystem editing tools. The Rust Hub validates proposed exact replacements and selected paths before applying them.

## Verification

```sh
uv pip install --python services/spark-mlx/.venv/bin/python pytest httpx
services/spark-mlx/.venv/bin/python -m pytest services/spark-mlx/test_server.py -q
cargo run -p hub-runtime --example spark_fixture
```

The second command group requires the live service. It performs route → edit proposal → first-pass review → exact file verification → progress summary in a temporary fixture, without Codex.

Sources: https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit and https://github.com/XHToken/Spark-MLX-LLM . Quantized-model performance is not inferred from the unquantized model's published benchmarks.
