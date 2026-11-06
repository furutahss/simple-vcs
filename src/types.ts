export type Target = { kind: "local" } | { kind: "ssh"; host: string };
export type VcsKind = "svn" | "git";
export interface Workspace { id: string; name: string; target: Target; path: string; vcs: VcsKind }
export interface Settings { version: number; workspaces: Workspace[]; svn: string; git: string; ssh: string }
export interface GitChange { path: string; index: string; worktree: string; untracked: boolean; conflicted: boolean; submodule: boolean }
export interface GitCommit { hash: string; parents: string[]; author: string; date: string; message: string; refs: string }
export interface GitBranch { name: string; current: boolean; upstream: string; ahead: number; behind: number }
export interface GitInfo { version: string; root: string; branch: string; head: string; detached: boolean; remote: string; operation: string; warnings: string[] }
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
  configure(svn: string, git: string, ssh: string): Promise<Settings>;
  inspect(id: string): Promise<Info>;
  status(id: string, includeUnchanged?: boolean): Promise<Change[]>;
  diff(id: string, path: string, revision?: number): Promise<string>;
  history(id: string, before?: number): Promise<Revision[]>;
  gitInfo(id: string): Promise<GitInfo>;
  gitStatus(id: string): Promise<GitChange[]>;
  gitDiff(id: string, path: string, staged: boolean): Promise<string>;
  gitHistory(id: string, skip: number): Promise<GitCommit[]>;
  gitCommitDiff(id: string, hash: string): Promise<string>;
  gitBranches(id: string): Promise<GitBranch[]>;
  gitWrite(id: string, action: GitWriteAction, paths?: string[], message?: string): Promise<string>;
  chooseDirectory(): Promise<string | null>;
  previewWrite(id: string, request: WriteRequest): Promise<WritePreview>;
  executeWrite(id: string, token: string): Promise<WriteResult>;
  pendingOperations(): Promise<PendingOperation[]>;
  inspectPending(operationId: string): Promise<RecoveryInfo>;
  acknowledgePending(operationId: string, confirmed: boolean): Promise<void>;
}
export type WriteAction = "add" | "delete" | "commit" | "update" | "revert" | "cleanup" | "resolve";
export type GitWriteAction = "stage" | "unstage" | "discard" | "commit" | "branch" | "switch" | "fetch" | "pull" | "push";
export interface WriteRequest { action: WriteAction; paths: string[]; message: string }
export interface WritePreview { token: string; workspace: Workspace; action: WriteAction; paths: string[]; warnings: string[] }
export interface WriteResult { output: string; revision: number | null }
export interface PendingOperation { id: string; workspace: Workspace; action: WriteAction | GitWriteAction; paths: string[]; startedAt: number }
export interface RecoveryInfo { operation: PendingOperation; changes: Change[]; history: Revision[]; gitHistory: GitCommit[] }
