import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "./App";
import type { Api, Settings, Workspace } from "./types";

const local: Workspace = { id: "local", name: "ローカル開発", target: { kind: "local" }, path: "/tmp/work", vcs: "svn" };
const remote: Workspace = { id: "remote", name: "Linux開発", target: { kind: "ssh", host: "team" }, path: "/srv/work", vcs: "svn" };
const config: Settings = { version: 1, workspaces: [local, remote], svn: "svn", git: "git", ssh: "ssh" };
function api(): Api {
  return {
    load: vi.fn().mockResolvedValue(config), save: vi.fn().mockResolvedValue(config),
    remove: vi.fn().mockResolvedValue(config), configure: vi.fn().mockResolvedValue(config),
    inspect: vi.fn().mockResolvedValue({ root: "/tmp/work", url: "file:///repo", revision: 2, svnVersion: "1.15", warnings: [] }),
    status: vi.fn().mockResolvedValue([{ path: "日本語.txt", status: "modified", properties: "none", treeConflicted: false, locked: false, copied: false, switched: false }]),
    diff: vi.fn().mockResolvedValue("--- before\n+++ after\n-old\n+new"),
    history: vi.fn().mockResolvedValue([{ revision: 2, author: "alice", date: "2026-11-06T01:00:00Z", message: "変更内容", paths: [{ action: "M", path: "/trunk/a" }] }]),
    chooseDirectory: vi.fn().mockResolvedValue("/tmp/selected"),
    gitInfo: vi.fn(), gitStatus: vi.fn(), gitDiff: vi.fn(), gitHistory: vi.fn(), gitBranches: vi.fn(), gitCommitDiff: vi.fn(), gitWrite: vi.fn(),
    previewWrite: vi.fn(), executeWrite: vi.fn(), pendingOperations: vi.fn().mockResolvedValue([]), inspectPending: vi.fn(), acknowledgePending: vi.fn(),
  };
}

describe("SVN閲覧", () => {
  it("ウィンドウ復帰時の再取得で有効な選択が消えない", async () => {
    const client = api();
    render(<App client={client} />);
    await userEvent.click(await screen.findByLabelText("日本語.txt を選択"));
    await act(async () => { window.dispatchEvent(new Event("focus")); });
    await waitFor(() => expect(client.status).toHaveBeenCalledTimes(2));
    await waitFor(() => expect((screen.getByLabelText("日本語.txt を選択") as HTMLInputElement).checked).toBe(true));
    await userEvent.click(screen.getByRole("button", { name: /Linux開発/ }));
    await waitFor(() => expect((screen.getByLabelText("日本語.txt を選択") as HTMLInputElement).checked).toBe(false));
  });
  it("通常ファイルも表示して削除対象を選べる", async () => {
    const client = api();
    client.status = vi.fn().mockImplementation((_id: string, include?: boolean) => Promise.resolve(include ? [{ path: "normal.txt", status: "normal", properties: "none", treeConflicted: false, locked: false, copied: false, switched: false }] : []));
    render(<App client={client} />);
    await screen.findByText("変更はありません");
    await userEvent.click(screen.getByLabelText("通常ファイルも表示"));
    await userEvent.click(await screen.findByLabelText("normal.txt を選択"));
    expect((screen.getByRole("button", { name: "削除予定にする" }) as HTMLButtonElement).disabled).toBe(false);
    expect(client.status).toHaveBeenLastCalledWith("local", true);
  });
  it("左右キーで表示タブを切り替えられる", async () => {
    render(<App client={api()} />);
    await screen.findByRole("button", { name: /日本語.txt/ });
    await userEvent.click(screen.getByRole("tab", { name: /変更/ }));
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "履歴" }).getAttribute("aria-selected")).toBe("true");
    expect(await screen.findByRole("button", { name: /r2.*変更内容/ })).toBeTruthy();
  });
  it("取得中に同じ作業コピーを選んでも処理が取り残されない", async () => {
    const client = api();
    let finish!: (value: never[]) => void;
    client.status = vi.fn().mockImplementation(() => new Promise<never[]>(resolve => { finish = resolve; }));
    render(<App client={client} />);
    await userEvent.click(await screen.findByRole("button", { name: /ローカル開発.*ローカル/ }));
    await act(async () => { finish([]); });
    expect(await screen.findByText("変更はありません")).toBeTruthy();
  });
  it("選択中の作業コピーを再選択しても取得結果が消えない", async () => {
    const client = api();
    render(<App client={client} />);
    await screen.findByRole("button", { name: /日本語.txt/ });
    await userEvent.click(screen.getByRole("button", { name: /ローカル開発.*ローカル/ }));
    await userEvent.click(screen.getByRole("button", { name: /日本語.txt/ }));
    expect(await screen.findByText("+new")).toBeTruthy();
  });

  it("履歴の続きは最後に取得したリビジョンの直前から取得する", async () => {
    const client = api();
    client.history = vi.fn().mockImplementation((_id: string, before?: number) => Promise.resolve(before === undefined
      ? Array.from({ length: 100 }, (_, index) => ({ revision: 200 - index, author: "alice", date: "", message: `履歴${200 - index}`, paths: [] }))
      : [{ revision: 100, author: "alice", date: "", message: "続きの履歴", paths: [] }]));
    render(<App client={client} />);
    await userEvent.click(await screen.findByRole("tab", { name: "履歴" }));
    await userEvent.click(await screen.findByRole("button", { name: "次の100件" }));
    expect(await screen.findByRole("button", { name: /r100.*続きの履歴/ })).toBeTruthy();
    expect(client.history).toHaveBeenLastCalledWith("local", 100);
  });
  it("選択した変更の差分を取得して表示する", async () => {
    const client = api();
    render(<App client={client} />);
    await userEvent.click(await screen.findByRole("button", { name: /日本語.txt/ }));
    expect(await screen.findByText("+new")).toBeTruthy();
    expect(client.diff).toHaveBeenCalledWith("local", "日本語.txt");
  });

  it("SSH先に切り替えた後に古い差分が到着しても表示しない", async () => {
    const client = api();
    let finish!: (value: string) => void;
    client.diff = vi.fn().mockImplementation(() => new Promise<string>(resolve => { finish = resolve; }));
    render(<App client={client} />);
    await userEvent.click(await screen.findByRole("button", { name: /日本語.txt/ }));
    await userEvent.click(screen.getByRole("button", { name: /Linux開発/ }));
    await act(async () => { finish("OLD_RESULT"); });
    await waitFor(() => expect(screen.queryByText("OLD_RESULT")).toBeNull());
    expect(screen.getByText("team")).toBeTruthy();
  });

  it("状態取得に失敗したら変更なしと表示しない", async () => {
    const client = api();
    client.status = vi.fn().mockRejectedValue({ kind: "authentication", message: "鍵を確認してください" });
    render(<App client={client} />);
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", expect.stringContaining("鍵を確認してください"));
    expect(screen.queryByText("変更はありません")).toBeNull();
  });

  it("登録画面でローカルとSSH先を選べる", async () => {
    const client = api();
    render(<App client={client} />);
    await userEvent.click(await screen.findByRole("button", { name: "作業コピーを登録" }));
    expect(screen.queryByLabelText("SSH Host別名")).toBeNull();
    await userEvent.selectOptions(screen.getByLabelText("実行先"), "ssh");
    expect(screen.getByLabelText("SSH Host別名")).toBeTruthy();
  });

  it("履歴を選び、変更パスとリビジョンの差分を表示する", async () => {
    const client = api();
    render(<App client={client} />);
    await userEvent.click(await screen.findByRole("tab", { name: "履歴" }));
    await userEvent.click(await screen.findByRole("button", { name: /r2.*変更内容/ }));
    await userEvent.click(screen.getByRole("button", { name: /\/trunk\/a/ }));
    expect(await screen.findByText("+new")).toBeTruthy();
    expect(client.diff).toHaveBeenCalledWith("local", "/trunk/a", 2);
  });
});
