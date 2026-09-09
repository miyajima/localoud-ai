# 本地MLX服务（默认：Spark-X2.5-4B-MLX-8bit）

[English](README.md) | [日本語](README.ja.md) | 简体中文

服务使用固定在`model-revision.txt`中的revision对应的社区MLX 8bit转换版`abenzerps/Spark-X2.5-4B-MLX-8bit`，并在`requirements.txt`中固定XHToken的架构适配器。不会执行下载模型中的Python代码。加载前会检查模型权重的SHA-256。软件包的strict tensor loading和实际生成测试是分开的检查。

```sh
uv venv services/spark-mlx/.venv --python 3.12
uv pip install --python services/spark-mlx/.venv/bin/python -r services/spark-mlx/requirements.txt
services/spark-mlx/.venv/bin/python services/spark-mlx/download_model.py
services/spark-mlx/.venv/bin/python services/spark-mlx/server.py
```

在macOS上，请从能够看到Metal的会话启动服务。服务只绑定到`127.0.0.1:8765`。使用`SPARK_PORT`覆盖端口，使用`SPARK_MODEL_PATH`覆盖checkpoint目录。浏览器发来的host请求和非loopback主机都会被拒绝。

- `GET /health`：实际加载的模型、配置的量化方式、设备和revision。
- `POST /v1/generate`：`model`（不匹配会在推理前拒绝）、`task`、`messages`、JSON `schema`、`max_tokens`、`temperature`。
- 支持的task：`difficulty`（五级评估）、`route`、`draft_context`、`implement`、`summarize`、`review`、`retrieval_query`、`memory_extract`。Reasoning由本地服务固定，不支持按请求切换。
- 输出JSON会被解析并验证。这是生成后的schema验证，不代表使用了语法约束解码。格式错误的输出以422拒绝；推理繁忙时返回429。Rust provider会串行化进程内调用，避免MLX请求重叠。验证错误只返回长度受限的schema诊断，不会暴露生成的payload。
- 由一个专用推理thread负责MLX。输入上限为8192 token，输出上限为4096 token。
- 服务没有shell或文件编辑工具。Rust Hub会在应用前验证提议的完全匹配替换和选定路径。

## 验证

```sh
uv pip install --python services/spark-mlx/.venv/bin/python pytest httpx
services/spark-mlx/.venv/bin/python -m pytest services/spark-mlx/test_server.py -q
cargo run -p hub-runtime --example spark_fixture
```

第二组命令需要正在运行的服务。它不使用Codex，而是在临时fixture中执行route → 编辑提案 → 首轮审查 → 完全匹配的文件验证 → 进度摘要。

来源：[Hugging Face上的Spark模型](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)、[Spark-MLX-LLM](https://github.com/XHToken/Spark-MLX-LLM)。不会根据未量化模型公开的基准推断量化模型性能。

## 替换模型

准备包含`model_id`、`path`、`loader`（`spark`或`mlx_lm`）和`quantization_bits`的JSON文件，并将其设置到`LOCAL_MODEL_CONFIG`。未设置时使用固定的Spark 8bit配置。完整步骤和HTTP适配器契约请参阅[模型配置](../../docs/MODEL_CONFIGURATION.md)。其他MLX-LM模型会检查checkpoint配置、shard和strict loading，但不会继承Spark发行版的checksum验证或质量证据。
