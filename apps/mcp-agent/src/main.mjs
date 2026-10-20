import {
  app,
  BrowserWindow,
  ipcMain,
  dialog,
  shell,
  clipboard,
  Menu,
  Tray,
  nativeImage,
} from "electron";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { readFile, lstat, mkdir } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { AgentController } from "./controller.mjs";
import { ApprovalWindows } from "./approval-windows.mjs";
import { trustedIpc, allowApprovalNavigation } from "./security.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const renderer = pathToFileURL(join(here, "renderer.html")).href;
let window,
  tray,
  controller,
  quitting = false;
const unique = app.requestSingleInstanceLock();
if (!unique) app.quit();
app.on("second-instance", () => showMain());
function showMain() {
  if (window && !window.isDestroyed()) {
    window.show();
    window.focus();
  }
}
function restrictWindow(win, allowed) {
  win.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  win.webContents.on("will-navigate", (event, url) => {
    if (!allowed(url)) event.preventDefault();
  });
  win.webContents.on("will-attach-webview", (event) => event.preventDefault());
  win.webContents.session.setPermissionRequestHandler(
    (_web, _permission, callback) => callback(false),
  );
  win.webContents.session.setPermissionCheckHandler(() => false);
}
const approvals = new ApprovalWindows({
  getAgent: () => controller?.agent,
  readSession: async () => {
    const tokenFile = join(controller.dataDir, "agent", "ui-token");
    const stat = await lstat(tokenFile);
    if (
      !stat.isFile() ||
      stat.isSymbolicLink() ||
      stat.nlink !== 1 ||
      (stat.mode & 0o777) !== 0o600 ||
      stat.uid !== process.getuid()
    )
      throw Error("unsafe_local_state");
    const token = (await readFile(tokenFile, "utf8")).trim();
    if (!/^[a-f0-9]{64}$/.test(token)) throw Error("invalid_local_session");
    return token;
  },
  makeWindow: (origin) => {
    const win = new BrowserWindow({
      title: "Localoud — 操作の承認",
      width: 1040,
      height: 800,
      minWidth: 640,
      minHeight: 520,
      show: false,
      webPreferences: {
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
        partition: "localoud-approval-" + randomUUID(),
        webSecurity: true,
      },
    });
    restrictWindow(win, (url) => allowApprovalNavigation(url, origin));
    return {
      isDestroyed: () => win.isDestroyed(),
      destroy: () => win.destroy(),
      show: () => win.show(),
      focus: () => win.focus(),
      setSession: (token) =>
        win.webContents.session.cookies.set({
          url: origin,
          name: "localoud_session",
          value: token,
          httpOnly: true,
          sameSite: "strict",
          path: "/",
        }),
      load: (url) => win.loadURL(url),
    };
  },
});
const showApprovals = () => approvals.open();
const closeApprovals = () => approvals.close();
function register(name, handler) {
  ipcMain.handle("localoud:" + name, async (event, ...args) => {
    const frame = event.senderFrame;
    if (
      event.sender !== window?.webContents ||
      !trustedIpc(
        { url: frame?.url, mainFrame: frame === window.webContents.mainFrame },
        renderer,
      )
    )
      return { ok: false, error: "untrusted_sender" };
    try {
      return { ok: true, value: await handler(...args) };
    } catch (error) {
      const allowed = [
        "setup_required",
        "invalid_options",
        "invalid_folder",
        "private_state_overlaps_project",
        "unsafe_local_state",
        "platform_not_supported",
        "agent_not_running",
        "project_overlap",
        "project_limit",
        "unknown_project",
        "project_unavailable",
      ];
      return {
        ok: false,
        error: allowed.includes(error.message)
          ? error.message
          : "operation_failed",
      };
    }
  });
}
async function initialize() {
  const userData = app.getPath("userData");
  await mkdir(userData, { recursive: true });
  const resources = app.isPackaged
    ? process.resourcesPath
    : resolve(here, "../../..");
  const agentModule = app.isPackaged
    ? join(resources, "agent", "runtime.mjs")
    : join(resources, "plugins", "localoud-mcp", "agent", "runtime.mjs");
  const binary = app.isPackaged
    ? join(resources, "bin", "localoud-mcp")
    : join(resources, "target", "release", "localoud-mcp");
  const { startAgent } = await import(pathToFileURL(agentModule).href);
  controller = new AgentController({
    dataDir: join(userData, "private"),
    binary,
    startAgent,
  });
  await controller.initialize();
  controller.startAtLogin = app.getLoginItemSettings().openAtLogin;
  window = new BrowserWindow({
    title: "Localoud Agent",
    width: 1080,
    height: 830,
    minWidth: 760,
    minHeight: 600,
    show: false,
    backgroundColor: "#edf4fa",
    webPreferences: {
      preload: join(here, "preload.cjs"),
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
      webSecurity: true,
    },
  });
  restrictWindow(window, (url) => url === renderer);
  window.on("close", (event) => {
    if (!quitting) {
      event.preventDefault();
      window.hide();
    }
  });
  register("state", () => controller.state());
  register("choose-folders", async () => {
    const result = await dialog.showOpenDialog(window, {
      title: "ChatGPTで使うプロジェクトを追加",
      properties: ["openDirectory", "createDirectory", "multiSelections"],
    });
    if (result.canceled) return controller.state();
    closeApprovals();
    return controller.addFolders(result.filePaths);
  });
  register("update-project", async (options) => {
    closeApprovals();
    return controller.updateProject(options);
  });
  register("remove-project", async (id) => {
    closeApprovals();
    return controller.removeProject(id);
  });
  register("save-projects", async () => {
    closeApprovals();
    return controller.saveProjects();
  });
  register("recheck-projects", () => controller.recheckProjects());
  register("start", () => controller.start());
  register("stop", async () => {
    closeApprovals();
    return controller.stop();
  });
  register("open-chatgpt", async () => {
    await shell.openExternal("https://chatgpt.com/plugins");
    return true;
  });
  register("copy-url", () => {
    clipboard.writeText(controller.state().mcpUrl);
    return true;
  });
  register("show-approvals", async () => {
    await showApprovals();
    return true;
  });
  register("set-login", (value) => {
    if (typeof value !== "boolean") throw Error("invalid_options");
    app.setLoginItemSettings({ openAtLogin: value, openAsHidden: true });
    controller.startAtLogin = app.getLoginItemSettings().openAtLogin;
    return controller.state();
  });
  const iconPath = app.isPackaged
    ? join(resources, "icon.png")
    : resolve(here, "../assets/icon.png");
  const icon = nativeImage
    .createFromPath(iconPath)
    .resize({ width: 18, height: 18 });
  icon.setTemplateImage(true);
  tray = new Tray(icon);
  tray.setToolTip("Localoud Agent");
  tray.setContextMenu(
    Menu.buildFromTemplate([
      { label: "Localoudを開く", click: showMain },
      {
        label: "操作の承認",
        click: () => showApprovals().catch(() => showMain()),
      },
      { type: "separator" },
      { label: "終了（接続を停止）", click: () => app.quit() },
    ]),
  );
  tray.on("click", showMain);
  Menu.setApplicationMenu(
    Menu.buildFromTemplate([
      {
        label: "Localoud Agent",
        submenu: [
          { label: "Localoudを開く", click: showMain },
          { role: "about" },
          { type: "separator" },
          {
            label: "終了（接続を停止）",
            accelerator: "CmdOrCtrl+Q",
            click: () => app.quit(),
          },
        ],
      },
      { role: "editMenu" },
      { role: "windowMenu" },
    ]),
  );
  await window.loadURL(renderer);
  window.show();
  if (controller.state().configured) controller.start().catch(() => {});
}
app.on("activate", showMain);
app.on("window-all-closed", () => {});
app.on("before-quit", (event) => {
  if (quitting) return;
  event.preventDefault();
  quitting = true;
  closeApprovals();
  Promise.resolve(controller?.stop()).finally(() => app.quit());
});
if (unique)
  app
    .whenReady()
    .then(initialize)
    .catch(() => {
      dialog.showErrorBox(
        "Localoudを起動できません",
        "アプリ内の実行ファイル、設定の権限、対応OSを確認してください。Windows版は未対応です。",
      );
      quitting = true;
      app.quit();
    });
