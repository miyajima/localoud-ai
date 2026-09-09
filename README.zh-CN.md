# Localoud AI

[English](README.md) | [日本語](README.ja.md) | 简体中文

Localoud是Local与Cloud的组合名称。Localoud AI是开源的桌面编码工作区。

这是一个本地优先的桌面编码工作区。Rust负责生命周期和持久化，Tauri提供工作区界面。Codex app-server在隔离的Git worktree中执行编码任务。[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)负责本地路由、小规模编辑、摘要和提取；Astra可按需规划和审查结果。worker从明确的上下文胶囊开始，不会隐式复制父会话。

## 支持开发 ☕

Localoud AI 是免费开源软件。如果它让你的开发工作更轻松，欢迎[请我喝杯咖啡](https://buymeacoffee.com/miyajima)。你的支持将用于开发、模型测试和持续改进。支持完全自愿。

## 为什么选择 Localoud

Localoud把日常、范围明确的编码工作留在本机处理，同时为规模更大或更复杂的工作提供清晰的Codex和Astra路径。你可以查看当前选择的模型，把改动限制在隔离的worktree中，在本地保存任务状态，并在应用改动前审查边界。

## 适合谁

- 希望使用本地推理完成路由、小规模编辑、摘要和提取的开发者。
- 需要明确任务范围、持久化历史、worktree隔离和可审查改动的团队。
- 想使用实用本地编码模型的Mac用户。目前推荐[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)。

## 主要功能

- Chat用于直接任务，Plan用于有依赖关系的DAG调度和重启，Agents用于审查和返工。
- 带有上限pull和来源信息的上下文胶囊，以及记录实测模型token的Usage。
- 明确选择模型和Reasoning，五级自动路由，以及可替换的本地模型配置。
- Codex任务的Git worktree隔离，本地编辑的精确路径和精确替换验证，以及可见的审批边界。
- 使用SQLite保存项目、会话、草稿、计划、用量和重启状态，并提供安全Markdown渲染和易读的diff。

## 一条命令安装（macOS）

安装Rust stable、Node 22.22.2以上（或24.15以上 / 26以上）和Xcode Command Line Tools后，在代码检出目录运行这一条命令：

```sh
./scripts/install.sh
```

该脚本会安装前端依赖，并将未签名的调试版`Localoud AI.app`构建到`target/debug/bundle/macos/Localoud AI.app`。脚本不会下载Spark模型权重；需要本地推理时，请单独启动[Spark服务](services/spark-mlx/README.zh-CN.md)。如果要从GitHub克隆并在一行中完成安装，可运行：

```sh
git clone https://github.com/miyajima/localoud-ai.git && cd localoud-ai && ./scripts/install.sh
```

## 开发

需要Rust stable、Node 22.22.2以上（或24.15以上 / 26以上）；macOS还需要Xcode Command Line Tools。

```sh
cd apps/desktop
npm ci
npm run tauri dev
```

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)已配置为**8bit**。模型和运行时的安装与实际provider执行是分开的验收检查。

请参阅[实现状态](docs/IMPLEMENTATION_STATUS.md)和[修订版计划](docs/ASTRA_CODEX_HUB_PLAN.md)。

## 运行桌面应用

已验证的调试包位于`target/debug/bundle/macos/Localoud AI.app`。打开它，注册Git仓库，并在Settings中设置Codex二进制文件的绝对路径。进行本地运行时，请单独启动[Spark服务](services/spark-mlx/README.zh-CN.md)。Astra可在Settings中选择启用。OrgBrain凭据是可选的，并通过macOS Keychain保存。没有预先配置真实的OrgBrain连接。

Chat用于处理任务，Plan用于DAG调度和重启，Agents用于审查和返工，Context用于上下文胶囊、pull预算和记忆来源，Usage用于查看实测模型token。`config.example.toml`仅用于说明；实际设置通过界面保存到SQLite。当前需要审查的provider操作会显示错误并停止。分支合并由用户执行。

[上下文实测基准](docs/CONTEXT_BENCHMARK.md)包含一次真实配对比较及其限制。完整验收情况以及可选或未验证的集成见[实现状态](docs/IMPLEMENTATION_STATUS.md)。

本地LLM实测值请参阅[本地LLM编码评估](docs/LOCAL_LLM_BENCHMARK.zh-CN.md)。该结果仅限于独自设计的20道编码专用题；reasoning开启与关闭使用不同的token和时间上限，因此平均时间不能作为同一条件下的直接比较。

目前首选是**[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)**。测量环境为**M5 MacBook Pro（48GB内存）**。预计16GB内存的MacBook也能充分运行，但尚未在16GB环境中实测。

## 桌面UX

使用**⌘O**或侧边栏的＋打开项目文件夹。**⌘N**新建任务，**⌘K**搜索任务、项目和操作，**⌘,**打开Settings，**⌘B**切换侧边栏。使用**⌘Enter**发送，Enter用于换行。草稿、上次工作区和已置顶任务会在本地恢复。

[UX比较](docs/UX_COMPARISON.md)列出了24项差异和已采用或部分采用的19项改进，包括原生文件夹/文件选择、重命名/搜索/置顶、安全Markdown、易读的diff和计划预览。注册项目仍需要已有的Git仓库。

模型名称、Reasoning选择和本地模型替换请参阅[MODEL_CONFIGURATION.md](docs/MODEL_CONFIGURATION.md)；五级自动路由请参阅[AUTO_ROUTING.md](docs/AUTO_ROUTING.md)。

输入框的斜杠命令、提及、计划模式和历史补全见[COMPOSER.md](docs/COMPOSER.md)。

[ChatGPT连接与会话管理](docs/CHATGPT.md)介绍独立浏览器provider的设置、配额边界和剩余的实时验证工作。

## 本地设置和凭据

`config.example.toml`和`services/spark-mlx/config.yaml`是公开示例和默认值，不是个人设置。请在应用中配置自己的provider。运行时设置、会话和草稿保存在本地应用数据库中；OrgBrain凭据使用macOS Keychain。只读MCP的Bearer token保存在macOS Keychain中。ChatGPT使用独立的持久化WKWebView数据存储。不要提交数据库文件、本地配置、provider凭据或MCP Bearer token。

## 许可证

Localoud AI使用[MIT License](LICENSE)。第三方依赖和单独下载的模型权重分别受其各自许可证约束。
