const { contextBridge, ipcRenderer } = require("electron");
const call = (method, ...args) =>
  ipcRenderer.invoke("localoud:" + method, ...args).then((result) => {
    if (!result.ok) throw Error(result.error);
    return result.value;
  });
contextBridge.exposeInMainWorld(
  "localoud",
  Object.freeze({
    state: () => call("state"),
    chooseFolders: () => call("choose-folders"),
    updateProject: (options) => call("update-project", options),
    removeProject: (id) => call("remove-project", id),
    saveProjects: () => call("save-projects"),
    recheckProjects: () => call("recheck-projects"),
    start: () => call("start"),
    stop: () => call("stop"),
    openChatGPT: () => call("open-chatgpt"),
    copyMcpUrl: () => call("copy-url"),
    showApprovals: () => call("show-approvals"),
    setStartAtLogin: (value) => call("set-login", value),
  }),
);
