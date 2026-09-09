# Astra ↔ Codex Handoff Architecture

更新日: 2026-09-08  
対象: ChatGPT Pro / ChatGPT Web (Astra) / read-only MCP / Tauri App / Codex

## 目的

ChatGPT WebのAstraをPlanner / Reviewer、CodexをImplementerとして使い、OpenAIの現行仕様・利用規約に抵触するリスクを避けながら、ユーザー操作を最小化する。

---

## アーキテクチャ

```text
┌──────────────────────────────┐
│ ChatGPT Web / Astra          │
│                              │
│ Planner / Reviewer           │
└──────────────┬───────────────┘
               │
               │ read-only MCP
               ▼
┌──────────────────────────────┐
│ Local MCP Server             │
│                              │
│ repo tree                    │
│ file read/search             │
│ git status                   │
│ git diff                     │
│ test results                 │
│ Codex task/session status    │
└──────────────┬───────────────┘
               │
               ▼
        Local Repository
               ▲
               │
               │ implementation
               │
┌──────────────┴───────────────┐
│ Codex                        │
│                              │
│ Implement / Test / Fix       │
└──────────────▲───────────────┘
               │
               │ task handoff
               │
┌──────────────┴───────────────┐
│ Tauri App                    │
│                              │
│ Clipboard read on user action│
│ Manifest validation          │
│ Project resolution           │
│ Codex start / resume         │
│ Session registry             │
│ Test result capture          │
└──────────────────────────────┘
```

### 役割分担

**Astra**
- MCPでローカルrepoを読む
- 実装計画を作る
- Codex用Task Manifestを生成する
- 実装後のgit diff / test結果をMCPで読む
- コードレビューする
- 必要ならReview Manifestを生成する

**Local MCP Server**
- read/fetch専用
- repo構成、ファイル、git diff、test結果などをAstraへ提供
- ファイル変更、shell実行、Codex起動などの副作用は持たない

**Tauri App**
- ユーザー操作時だけClipboardを読む
- Astraが生成したManifestを検証する
- 対象projectを解決する
- Codexの新規task開始 / 既存session resumeを行う
- task_idとCodex sessionを紐付ける
- test結果をローカルに保存する

**Codex**
- Manifestを受け取る
- repoを確認する
- 実装する
- test / lint等を実行する
- 必要なら修正する
- git diffとして結果を残す

---

## 規約上の境界

以下は行わない。

- ChatGPT WebのDOMを自動取得しない
- Playwright / Selenium / CDP等でChatGPT回答を取得しない
- ChatGPTの内部APIやnetwork responseを直接読まない
- ChatGPTのsession cookieや認証情報を利用しない
- MCPのread tool内部でファイル変更やCodex起動を行わない
- Clipboardを常時監視して自動実行しない

AstraからCodexへのhandoffは、**ユーザー自身がChatGPT UIでCopyした内容を、Tauri Appへ明示的に渡す**形にする。

---

# ユーザー手順

## 1. Astraで実装計画を作る

ChatGPT WebでAstraに依頼する。

例:

```text
このissueの実装計画を作って。
必要なrepo情報はMCPで確認して。
最後にCodex用Task Manifestを出して。
```

AstraはMCPで必要なファイル構成や既存コードを確認し、最後にTask Manifestを生成する。

---

## 2. ManifestをCodexへ送る

ChatGPTのCopyボタンでManifestをコピーする。

その後、Tauri Appの

```text
Send to Codex
```

または設定済みショートカットを実行する。

Tauri Appが自動で行う処理:

```text
Clipboard取得
↓
Manifest検証
↓
Project特定
↓
Task ID登録
↓
Codex起動
↓
新規session作成
```

通常の実装taskでは追加確認なしでCodexへ渡す。

危険操作を含む場合のみ確認を表示する。

---

## 3. Codexが実装する

Codexが以下を実行する。

```text
Manifest確認
↓
repo確認
↓
実装
↓
test / lint
↓
必要なら修正
↓
完了
```

実装結果はローカルrepoのgit diffとして残す。

test結果はTauri AppまたはCodex側からローカルartifactへ保存する。

---

## 4. Astraでレビューする

ChatGPT WebでAstraに伝える。

```text
Codexの実装をレビューして。
MCPでgit diffとtest結果を確認して。
```

AstraはMCPから以下を取得する。

```text
git status
git diff
git diff stat
test results
```

Codexの出力をChatGPTへコピーする必要はない。

---

## 5. 修正が必要な場合

AstraがReview Manifestを生成する。

ユーザーは再び:

```text
Copy
↓
Send to Codex
```

を実行する。

Tauri AppはTask IDから既存Codex sessionを特定し、同じsessionへReview Manifestを渡してresumeする。

---

# 最終UX

通常の1サイクルは以下。

```text
Astraに実装依頼
↓
AstraがMCPでrepo確認
↓
Task Manifest生成
↓
Copy
↓
Send to Codex
↓
Codexが実装・test
↓
Astraに「レビューして」
↓
AstraがMCPでdiff/test確認
↓
完了
```

修正が必要な場合:

```text
Astraレビュー
↓
Review Manifest生成
↓
Copy
↓
Send to Codex
↓
同じCodex sessionで修正
↓
Astra再レビュー
```

---

## 設計上のポイント

- AstraはPlanner / Reviewerに集中させる
- CodexはImplementerに集中させる
- MCPはread-onlyに限定する
- Tauri AppだけがhandoffとCodex session管理を担当する
- ChatGPT WebからのOutput取得は必ずユーザーの明示的Copyを境界にする
- 実装結果はコピーせず、AstraがMCPから直接読む
- 将来Full MCPが使える場合も、Task Manifest形式とsession管理はそのまま再利用する
