# Local implementation then Luna correction: preflight

2026-09-10 JST. Status: blocked before inference. No measured savings.

Requested comparison: local LLM implementation followed by Luna correction versus Luna implementation alone.

## Observed blocker

`http://127.0.0.1:8765/health` could not be reached. Starting the existing service with `services/spark-mlx/.venv/bin/python services/spark-mlx/server.py` exited with code 3 during application startup:

```text
ImportError: [metal::load_device] No Metal device available. This typically occurs in headless, sandboxed, or virtualized macOS sessions where the GPU is not accessible.
ERROR: Application startup failed. Exiting.
```

No local inference or Luna inference was attempted. No dependency installation, model download, or existing source modification was performed. Runtime model identity has not been verified; the repository default is `abenzerps/Spark-X2.5-4B-MLX-8bit`.

## Comparison protocol for resumption

- Freeze a self-contained implementation task, initial source and acceptance tests before inference; use separate temporary directories for each condition.
- A: Luna implements and fixes the initial source until acceptance passes.
- B: local model implements the same initial source, then Luna inspects and fixes that output until the same acceptance passes. Keep failed drafts and all correction usage.
- Use the same Luna execution path, model (`gpt-5.6-luna`), reasoning effort (proposed: `max`, matching the previous experiment), task specification and tools in both conditions. Record effective model and settings.
- Record each condition's cumulative input, cached input, uncached input, output, inference count, elapsed time and acceptance result. Record local usage separately; different tokenizers make cross-model token totals unsuitable as a cloud-cost measure.
- Primary comparison: Luna input plus output in B versus A, including inspection and correction. Also compare uncached input and output separately. Never infer subscription consumption or monetary savings from raw token counts.
- Bound the initial experiment to one paired task with a declared time/correction limit. Report any incomplete result without calling it savings. One successful pair remains preliminary, not general evidence.
- Preparation and parent verification/report usage are outside the measured implementation phases and must be disclosed as such.

The prior `docs/LUNA_PIPELINE_BENCHMARK.md` compares two Luna execution paths, not these two conditions, and cannot answer this request.

## Required environment

Start the existing local service from a normal macOS terminal where Metal is accessible:

```sh
cd /Users/miyajimakazuhiro/projects/localoud
services/spark-mlx/.venv/bin/python services/spark-mlx/server.py
```

Then recheck service readiness and model identity before any measured inference. Do not substitute another model silently.
