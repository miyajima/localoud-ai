# 本地LLM编码评估实测值

[日本語版](LOCAL_LLM_BENCHMARK.md) | 简体中文

本评估**仅限于独自设计的20道编码专用题**。它不是通用基准，也不保证代表编码工作的整体性能。以下是对已提供实测结果的整理；创建本文档时没有重新测量。

## 当前首选

目前首选是**[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)**。它的模型大小约4.37GB，在关闭reasoning时取得15/20，平均11.18秒，在大小、正确数和响应时间之间取得了平衡。

**预计16GB内存的MacBook也能充分运行，但尚未在16GB环境中实测。**运行时除了模型本体还会使用其他内存，因此可用余量会随上下文长度和同时运行的应用而变化。

## 测量条件与阅读方式

- 测量环境：**M5 MacBook Pro，48GB内存**。
- 关闭reasoning：总计 **4096 tokens**，上限 **180秒**。
- 开启reasoning：总计 **16384 tokens**，上限 **300秒**。
- 表中的结果表示“正确数 / 20道题，平均时间（秒）”。
- **reasoning开启与关闭使用不同的token和时间上限，因此平均时间不能作为同一条件下的直接比较。**
- “未测量”表示该条件没有实测结果。

## 结果

在本次测量中，**只有Qwen3.8-27B-MTPLX-Optimized-Speed取得了20/20正确**（关闭reasoning）。

| 模型 | 大小 | 关闭reasoning | 开启reasoning |
| --- | ---: | ---: | ---: |
| [Spark-X2.5-4B-MLX-4bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-4bit) | 约2.31GB | 2/20，7.98秒 | 3/20，83.52秒 |
| [Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit) | 约4.37GB | 15/20，11.18秒 | 16/20，95.78秒 |
| [Qwen3.8-27B-MTPLX-Optimized-Speed](https://huggingface.co/Youssofal/Qwen3.8-27B-MTPLX-Optimized-Speed) | 约20.70GB | 20/20，38.68秒 | 未测量 |
| [LFM2.5-2.6B-Uncensored-GGUF](https://huggingface.co/SC117/LFM2.5-2.6B-Uncensored-GGUF)（Q4_K_M） | 约1.67GB | 6/20，15.52秒 | 未测量 |
| [MiniCPM5-2B-GGUF](https://huggingface.co/openbmb/MiniCPM5-2B-GGUF)（Q8_0） | 约2.68GB | 12/20，8.14秒 | 未测量 |
| [MiniCPM5-2B-GGUF](https://huggingface.co/openbmb/MiniCPM5-2B-GGUF)（F16） | 约5.04GB | 14/20，10.79秒 | 未测量 |
| [Qwythos-9B-Claude-Mythos-5-1M-GGUF](https://huggingface.co/empero-ai/Qwythos-9B-Claude-Mythos-5-1M-GGUF)（MTP-Q4_K_M） | 约5.89GB | 15/20，20.93秒 | 未测量 |
| [Qwythos-9B-Claude-Mythos-5-1M-GGUF](https://huggingface.co/empero-ai/Qwythos-9B-Claude-Mythos-5-1M-GGUF)（MTP-Q8_0） | 约9.79GB | 17/20，25.22秒 | 未测量 |
| [Qwen3.5-4B-GGUF](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF)（Q8_0） | 约4.48GB | 18/20，23.42秒 | 未测量 |
| [Qwen3.5-4B-MTPLX-Optimized-Quality](https://huggingface.co/Youssofal/Qwen3.5-4B-MTPLX-Optimized-Quality)（8bit） | 约4.58GB | 17/20，13.91秒 | 未测量 |
| [Qwen3.5-4B-MTPLX-Optimized-Speed](https://huggingface.co/Youssofal/Qwen3.5-4B-MTPLX-Optimized-Speed)（4bit） | 约2.48GB | 13/20，14.08秒 | 未测量 |
| [gemma-4-12B-agentic-fable5-composer2.5-v2-3.5x-tau2-GGUF](https://huggingface.co/yuxinlu1/gemma-4-12B-agentic-fable5-composer2.5-v2-3.5x-tau2-GGUF)（v2 Q4_K_M） | 约7.38GB | 7/20，32.53秒 | 10/20，45.16秒 |
