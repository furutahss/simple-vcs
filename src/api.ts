import { invoke, isTauri } from "@tauri-apps/api/core";
import type { Api } from "./types";
function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return Promise.reject(new Error("デスクトップアプリで開いてください。起動コマンド: npm run tauri dev"));
  return invoke<T>(command, args);
}
export const api: Api = {
  load: () => call("load_workspaces"), save: workspace => call("save_workspace", { workspace }),
  remove: id => call("remove_workspace", { id }), configure: (svn, ssh) => call("configure_tools", { svn, ssh }),
  inspect: id => call("inspect_workspace", { id }), status: (id, includeUnchanged = false) => call("get_status", { id, includeUnchanged }),
  diff: (id, path, revision) => call("get_diff", { id, path, revision: revision ?? null }),
  history: (id, before) => call("get_history", { id, before: before ?? null }),
  chooseDirectory: () => call("choose_directory"),
  previewWrite: (id, request) => call("preview_write", { id, request }),
  executeWrite: (id, token) => call("execute_write", { id, token }),
  pendingOperations: () => call("get_pending_operations"),
  inspectPending: operationId => call("inspect_pending_operation", { operationId }),
  acknowledgePending: (operationId, confirmed) => call("acknowledge_pending_operation", { operationId, confirmed }),
};
