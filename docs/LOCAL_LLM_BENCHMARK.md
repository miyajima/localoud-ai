# ローカルLLM コーディング評価の実測値

[简体中文版](LOCAL_LLM_BENCHMARK.zh-CN.md)

この評価は、**独自のコーディング専用20問に絞った結果**です。汎用ベンチマークや、コーディング全般の性能を保証するものではありません。以下は提供された実測結果の掲載であり、このドキュメントの作成時に再測定は行っていません。

## 現時点のイチオシ

現時点のイチオシは **[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)** です。モデルサイズ約4.37GB、reasoningなしで15/20正解・平均11.18秒という、サイズ・正解数・応答時間のバランスを評価しています。

**16GBメモリのMacBookでも十分動作すると見込んでいますが、16GB環境での実測は未実施です。** 実行時にはモデル本体以外にもメモリを使用するため、コンテキスト長や同時に使うアプリによって余裕は変わります。

## 測定条件と読み方

- 測定環境：**M5 MacBook Pro、メモリ48GB**。
- reasoningなし：合計 **4096 tokens**・**180秒上限**。
- reasoningあり：合計 **16384 tokens**・**300秒上限**。
- 表の各結果は「正解数 / 20問、平均時間（秒）」を示します。
- **reasoning有無でトークン・時間上限が異なるため、平均時間は同一条件の直接比較ではありません。**
- 「未測定」は、その条件での実測結果がないことを示します。

## 結果

今回の測定では、**Qwen3.8-27B MTPLX Speedだけが20/20正解**でした（reasoningなし）。

| モデル | サイズ | reasoningなし | reasoningあり |
| --- | ---: | ---: | ---: |
| [Spark-X2.5-4B-MLX-4bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-4bit) | 約2.31GB | 2/20、7.98秒 | 3/20、83.52秒 |
| [Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit) | 約4.37GB | 15/20、11.18秒 | 16/20、95.78秒 |
| [Qwen3.8-27B-MTPLX-Optimized-Speed](https://huggingface.co/Youssofal/Qwen3.8-27B-MTPLX-Optimized-Speed) | 約20.70GB | 20/20、38.68秒 | 未測定 |
| [LFM2.5-2.6B-Uncensored-GGUF](https://huggingface.co/SC117/LFM2.5-2.6B-Uncensored-GGUF)（Q4_K_M） | 約1.67GB | 6/20、15.52秒 | 未測定 |
| [MiniCPM5-2B-GGUF](https://huggingface.co/openbmb/MiniCPM5-2B-GGUF)（Q8_0） | 約2.68GB | 12/20、8.14秒 | 未測定 |
| [MiniCPM5-2B-GGUF](https://huggingface.co/openbmb/MiniCPM5-2B-GGUF)（F16） | 約5.04GB | 14/20、10.79秒 | 未測定 |
| [Qwythos-9B-Claude-Mythos-5-1M-GGUF](https://huggingface.co/empero-ai/Qwythos-9B-Claude-Mythos-5-1M-GGUF)（MTP-Q4_K_M） | 約5.89GB | 15/20、20.93秒 | 未測定 |
| [Qwythos-9B-Claude-Mythos-5-1M-GGUF](https://huggingface.co/empero-ai/Qwythos-9B-Claude-Mythos-5-1M-GGUF)（MTP-Q8_0） | 約9.79GB | 17/20、25.22秒 | 未測定 |
| [Qwen3.5-4B-GGUF](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF)（Q8_0） | 約4.48GB | 18/20、23.42秒 | 未測定 |
| [Qwen3.5-4B-MTPLX-Optimized-Quality](https://huggingface.co/Youssofal/Qwen3.5-4B-MTPLX-Optimized-Quality)（8bit） | 約4.58GB | 17/20、13.91秒 | 未測定 |
| [Qwen3.5-4B-MTPLX-Optimized-Speed](https://huggingface.co/Youssofal/Qwen3.5-4B-MTPLX-Optimized-Speed)（4bit） | 約2.48GB | 13/20、14.08秒 | 未測定 |
| [gemma-4-12B-agentic-fable5-composer2.5-v2-3.5x-tau2-GGUF](https://huggingface.co/yuxinlu1/gemma-4-12B-agentic-fable5-composer2.5-v2-3.5x-tau2-GGUF)（v2 Q4_K_M） | 約7.38GB | 7/20、32.53秒 | 10/20、45.16秒 |
