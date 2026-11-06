export type Target = { kind: "local" } | { kind: "ssh"; host: string };
export interface Workspace { id: string; name: string; target: Target; path: string }
export interface Settings { version: number; workspaces: Workspace[]; svn: string; ssh: string }
export interface Change {
  path: string; status: string; properties: string; treeConflicted: boolean;
  locked: boolean; copied: boolean; switched: boolean;
}
export interface Info {
  root: string; url: string; revision: number; svnVersion: string; warnings: string[];
}
export interface Revision {
  revision: number; author: string; date: string; message: string;
  paths: { path: string; action: string }[];
}
export interface Api {
  load(): Promise<Settings>;
  save(workspace: Workspace): Promise<Settings>;
  remove(id: string): Promise<Settings>;
  configure(svn: string, ssh: string): Promise<Settings>;
  inspect(id: string): Promise<Info>;
  status(id: string, includeUnchanged?: boolean): Promise<Change[]>;
  diff(id: string, path: string, revision?: number): Promise<string>;
  history(id: string, before?: number): Promise<Revision[]>;
  chooseDirectory(): Promise<string | null>;
  previewWrite(id: string, request: WriteRequest): Promise<WritePreview>;
  executeWrite(id: string, token: string): Promise<WriteResult>;
  pendingOperations(): Promise<PendingOperation[]>;
  inspectPending(operationId: string): Promise<RecoveryInfo>;
  acknowledgePending(operationId: string, confirmed: boolean): Promise<void>;
}
export type WriteAction = "add" | "delete" | "commit" | "update" | "revert" | "cleanup" | "resolve";
export interface WriteRequest { action: WriteAction; paths: string[]; message: string }
export interface WritePreview { token: string; workspace: Workspace; action: WriteAction; paths: string[]; warnings: string[] }
export interface WriteResult { output: string; revision: number | null }
export interface PendingOperation { id: string; workspace: Workspace; action: WriteAction; paths: string[]; startedAt: number }
export interface RecoveryInfo { operation: PendingOperation; changes: Change[]; history: Revision[] }
