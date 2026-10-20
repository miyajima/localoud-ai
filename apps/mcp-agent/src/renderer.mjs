const MCP_URL_FALLBACK =
  "https://localoud-mcp-gateway.miyazima.workers.dev/mcp";
const bridge = window.localoud;

const elements = {
  error: document.querySelector("#error-banner"),
  notice: document.querySelector("#action-notice"),
  buildTarget: document.querySelector("#build-target"),
  connectionStatus: document.querySelector("#connection-status"),
  connectionStatusText: document.querySelector("#connection-status-text"),
  localNode: document.querySelector("#local-node"),
  localNodeState: document.querySelector("#local-node-state"),
  gatewayNode: document.querySelector("#gateway-node"),
  gatewayNodeState: document.querySelector("#gateway-node-state"),
  chatgptNode: document.querySelector("#chatgpt-node"),
  routeLinks: [...document.querySelectorAll(".route-link")],
  projectList: document.querySelector("#project-list"),
  recheckProjects: document.querySelector("#recheck-projects"),
  projectEmpty: document.querySelector("#project-empty"),
  projectCount: document.querySelector("#project-count"),
  scopeHelp: document.querySelector("#scope-help"),
  chooseFolder: document.querySelector("#choose-folder"),
  configure: document.querySelector("#configure"),
  savedState: document.querySelector("#saved-state"),
  startAgent: document.querySelector("#start-agent"),
  stopAgent: document.querySelector("#stop-agent"),
  showApprovals: document.querySelector("#show-approvals"),
  startAtLogin: document.querySelector("#start-at-login"),
  mcpUrl: document.querySelector("#mcp-url"),
  copyMcpUrl: document.querySelector("#copy-mcp-url"),
  openChatGPT: document.querySelector("#open-chatgpt"),
};

let currentState = null;
let actionInProgress = false;
const projectRows = new Map();
let loginSettingDirty = false;

const ERROR_COPY = {
  setup_required: "プロジェクトを追加し、設定を保存してから起動してください。",
  project_overlap:
    "登録済みのフォルダと重なっています。同じフォルダや、その親・子フォルダは重ねて登録できません。",
  project_limit: "登録できるプロジェクトは100件までです。",
  unknown_project: "プロジェクトが見つかりません。画面を更新してください。",
  project_unavailable:
    "確認できないプロジェクトがあります。フォルダへのアクセスを確認するか、登録から外して追加し直してください。操作履歴は保持されています。",
  invalid_options: "許可する操作の設定を確認してください。",
  invalid_folder:
    "選んだフォルダを確認できません。別のフォルダを選び直してください。",
  private_state_overlaps_project:
    "選んだフォルダがLocaloudの非公開設定領域と重なっています。別の作業フォルダを選んでください。",
  unsafe_local_state:
    "Localoudの非公開データのアクセス権を確認できません。設定を修復してから再起動してください。",
  platform_not_supported:
    "このビルドはmacOS arm64向けです。Windows版・Linux版は未対応です。",
  agent_not_running: "先にエージェントを起動してください。",
  operation_failed:
    "操作を完了できませんでした。設定と接続状態を確認して、もう一度お試しください。",
};

function callSafe(method, ...args) {
  if (!bridge || typeof bridge[method] !== "function") {
    throw new Error(
      "アプリ連携を読み込めません。Localoud Agentを再起動してください。",
    );
  }
  return bridge[method](...args);
}

function setText(element, value) {
  if (element.textContent !== value) element.textContent = value;
}

function errorMessage(error) {
  const code =
    typeof error === "string" ? error : error?.code || error?.message;
  if (typeof code === "string" && ERROR_COPY[code]) return ERROR_COPY[code];
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  return "操作を完了できませんでした。状態を確認して、もう一度お試しください。";
}

function showError(message) {
  setText(elements.error, message);
  elements.error.hidden = !message;
  elements.notice.hidden = true;
}

function showNotice(message) {
  setText(elements.notice, message);
  elements.notice.hidden = !message;
  elements.error.hidden = true;
}

function buildLabel(state) {
  if (state.platform === "darwin" && state.arch === "arm64")
    return "macOS · Apple silicon";
  if (state.platform === "darwin" && state.arch === "x64")
    return "macOS · Intel";
  if (state.platform === "darwin") return "macOS";
  if (state.platform === "win32") return "Windows版は未対応";
  if (state.platform === "linux") return "Linux版は未対応";
  return "対応環境を確認中";
}

function renderConnection(state) {
  const status = state.online
    ? "online"
    : state.running
      ? "running"
      : "offline";
  const label = state.online
    ? "クラウドに接続中"
    : state.running
      ? "起動中 · 接続を待っています"
      : state.configured
        ? "エージェント停止中"
        : "初期設定が必要です";

  elements.connectionStatus.dataset.state = status;
  setText(elements.connectionStatusText, label);

  elements.localNode.dataset.state = state.running ? "live" : "idle";
  elements.gatewayNode.dataset.state = state.online ? "live" : "idle";
  elements.chatgptNode.dataset.state = "idle";
  setText(
    elements.localNodeState,
    state.running ? "起動中" : state.configured ? "設定済み" : "未設定",
  );
  setText(
    elements.gatewayNodeState,
    state.online ? "接続中" : state.running ? "再接続待ち" : "待機中",
  );
  elements.routeLinks.forEach((link, index) =>
    link.classList.toggle("is-active", state.online && index === 0),
  );
}

function createProjectRow(project) {
  const node = document.createElement("section");
  node.className = "project-row";
  const heading = document.createElement("div");
  heading.className = "project-row-header";
  const name = document.createElement("h3");
  name.textContent = project.name;
  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "project-remove";
  remove.textContent = "登録から外す";
  remove.setAttribute("aria-label", project.name + "を登録から外す");
  const path = document.createElement("p");
  path.className = "folder-path";
  path.textContent = project.root;
  const unavailable = document.createElement("p");
  unavailable.className = "project-unavailable";
  unavailable.textContent = "フォルダが見つからないか、アクセスできません。";
  heading.append(name, remove);
  node.append(heading, path, unavailable);
  const group = document.createElement("fieldset");
  group.className = "permission-list";
  const legend = document.createElement("legend");
  legend.textContent = "このプロジェクトで許可する操作";
  group.append(legend);
  const checks = {};
  for (const [key, title, detail] of [
    [
      "writeEnabled",
      "ファイルの作成・編集",
      "変更内容をMacで確認してから反映します。",
    ],
    [
      "executionEnabled",
      "コマンドの実行",
      "コマンドごとにMacで確認してから実行します。",
    ],
  ]) {
    const label = document.createElement("label");
    label.className = "permission-option";
    const input = document.createElement("input");
    input.type = "checkbox";
    input.setAttribute("aria-label", project.name + "：" + title);
    const copy = document.createElement("span");
    copy.className = "permission-copy";
    const text = document.createElement("span");
    text.className = "permission-title";
    text.textContent = title;
    const description = document.createElement("span");
    description.className = "permission-detail";
    description.textContent = detail;
    copy.append(text, description);
    label.append(input, copy);
    group.append(label);
    checks[key] = input;
    input.addEventListener("change", () =>
      runAction(
        () =>
          callSafe("updateProject", {
            id: project.id,
            writeEnabled: checks.writeEnabled.checked,
            executionEnabled: checks.executionEnabled.checked,
          }),
        "変更した許可を保存してから起動してください。",
      ),
    );
  }
  remove.addEventListener("click", () =>
    runAction(
      () => callSafe("removeProject", project.id),
      "登録から外しました。変更を保存してください。フォルダ内のファイルは削除しません。",
    ),
  );
  node.append(group);
  return { node, checks, remove, unavailable };
}
function renderProjects(state) {
  const projects = state.projects || [],
    ids = new Set(projects.map((p) => p.id));
  for (const [id, row] of projectRows)
    if (!ids.has(id)) {
      row.node.remove();
      projectRows.delete(id);
    }
  projects.forEach((project, index) => {
    let row = projectRows.get(project.id);
    if (!row) {
      row = createProjectRow(project);
      projectRows.set(project.id, row);
    }
    if (elements.projectList.children[index] !== row.node)
      elements.projectList.insertBefore(
        row.node,
        elements.projectList.children[index] || null,
      );
    row.checks.writeEnabled.checked = project.writeEnabled === true;
    row.checks.executionEnabled.checked = project.executionEnabled === true;
    row.unavailable.hidden = project.available !== false;
  });
  elements.projectEmpty.hidden = projects.length > 0;
  setText(
    elements.projectCount,
    projects.length ? projects.length + "件のプロジェクト" : "未登録",
  );
}
function renderControls(state) {
  const running = state.running === true,
    busy = actionInProgress;
  elements.chooseFolder.disabled =
    busy || running || (state.projects?.length || 0) >= 100;
  for (const p of state.projects || []) {
    const row = projectRows.get(p.id);
    if (!row) continue;
    row.remove.disabled = busy || running;
    for (const input of Object.values(row.checks))
      input.disabled = busy || running || p.available === false;
  }
  elements.recheckProjects.hidden = !(state.projects || []).some(
    (p) => p.available === false,
  );
  elements.recheckProjects.disabled = busy || running;
  elements.configure.disabled = busy || running || !state.dirty;
  elements.startAgent.disabled = busy || running || !state.configured;
  elements.stopAgent.disabled = busy || !running;
  elements.stopAgent.hidden = !running;
  elements.startAgent.hidden = running;
  elements.showApprovals.disabled = busy || !running;
  elements.openChatGPT.disabled = busy || !running;
  elements.copyMcpUrl.disabled = busy;
  elements.startAtLogin.disabled = busy || !state.configured;
  const saved = !state.dirty && (state.projects?.length || 0) > 0;
  elements.savedState.dataset.state = saved ? "saved" : "unsaved";
  setText(
    elements.savedState,
    state.dirty
      ? "変更を保存してから起動してください"
      : saved
        ? "プロジェクト設定を保存しました"
        : "プロジェクトを追加してください",
  );
  setText(
    elements.scopeHelp,
    running
      ? "プロジェクトや許可を変更するには、先にエージェントを停止してください。"
      : "追加時は複数選択できます。登録から外しても、フォルダ内のファイルは削除されません。",
  );
}

function renderState(state) {
  if (!state || typeof state !== "object")
    throw new Error("アプリの状態を読み取れませんでした。");
  currentState = state;
  setText(elements.buildTarget, buildLabel(state));
  renderProjects(state);
  if (!loginSettingDirty && document.activeElement !== elements.startAtLogin) {
    elements.startAtLogin.checked = state.startAtLogin === true;
  }

  setText(
    elements.mcpUrl,
    typeof state.mcpUrl === "string" && state.mcpUrl
      ? state.mcpUrl
      : MCP_URL_FALLBACK,
  );
  renderConnection(state);
  renderControls(state);
  if (state.error) {
    elements.error.dataset.source = "state";
    showError(errorMessage(state.error));
  } else if (elements.error.dataset.source === "state") {
    elements.error.dataset.source = "";
    showError("");
  }
}

async function refreshState() {
  if (!bridge || typeof bridge.state !== "function") {
    showError(
      "Localoud Agentの接続を確認できません。アプリを再起動してください。",
    );
    return;
  }
  try {
    const state = await callSafe("state");
    renderState(state);
  } catch (error) {
    elements.error.dataset.source = "state";
    showError("状態を読み込めませんでした。" + " " + errorMessage(error));
  }
}

async function runAction(action, successMessage) {
  if (actionInProgress) return false;
  actionInProgress = true;
  elements.error.dataset.source = "action";
  showError("");
  renderControls(currentState || {});
  try {
    const result = await action();
    await refreshState();
    if (!elements.error.hidden) return false;
    const message =
      typeof successMessage === "function"
        ? successMessage(result, currentState)
        : successMessage;
    if (message) showNotice(message);
    if (currentState) renderControls(currentState);
    return true;
  } catch (error) {
    elements.error.dataset.source = "action";
    showError(errorMessage(error));
    return false;
  } finally {
    actionInProgress = false;
    if (currentState) renderControls(currentState);
  }
}

elements.chooseFolder.addEventListener("click", () => {
  const before = currentState?.projects?.length || 0;
  return runAction(
    () => callSafe("chooseFolders"),
    (_result, state) =>
      (state?.projects?.length || 0) > before
        ? "プロジェクトを追加しました。許可を確認して保存してください。"
        : "",
  );
});
elements.recheckProjects.addEventListener("click", () =>
  runAction(
    () => callSafe("recheckProjects"),
    "フォルダの状態を確認しました。",
  ),
);
elements.configure.addEventListener("click", () =>
  runAction(
    () => callSafe("saveProjects"),
    "設定を保存しました。次にエージェントを起動できます。",
  ),
);

elements.startAgent.addEventListener("click", () =>
  runAction(async () => {
    await callSafe("start");
  }, "エージェントを起動しました。接続状態を確認しています。"),
);

elements.stopAgent.addEventListener("click", () =>
  runAction(async () => {
    await callSafe("stop");
  }, "エージェントを停止しました。"),
);

elements.showApprovals.addEventListener("click", () =>
  runAction(() => callSafe("showApprovals"), "接続・承認画面を開きました。"),
);

elements.startAtLogin.addEventListener("change", async () => {
  const previous = currentState?.startAtLogin === true;
  const desired = elements.startAtLogin.checked;
  loginSettingDirty = true;
  const succeeded = await runAction(
    () => callSafe("setStartAtLogin", desired),
    desired
      ? "ログイン時に起動する設定にしました。"
      : "ログイン時の自動起動を解除しました。",
  );
  loginSettingDirty = false;
  elements.startAtLogin.checked = succeeded
    ? currentState?.startAtLogin === true
    : previous;
  if (currentState) renderControls(currentState);
});

elements.copyMcpUrl.addEventListener("click", () =>
  runAction(() => callSafe("copyMcpUrl"), "MCP URLをコピーしました。"),
);

elements.openChatGPT.addEventListener("click", () =>
  runAction(
    () => callSafe("openChatGPT"),
    "ブラウザでChatGPTの接続画面を開きました。",
  ),
);

function scheduleRefresh() {
  window.setTimeout(async () => {
    await refreshState();
    scheduleRefresh();
  }, 2000);
}

void refreshState().then(scheduleRefresh);
