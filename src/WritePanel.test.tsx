import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { WritePanel, RecoveryPanel } from "./WritePanel";
import type { Api, PendingOperation, Workspace, WriteRequest } from "./types";

const workspace: Workspace = { id: "one", name: "作業", target: { kind: "ssh", host: "linux" }, path: "/srv/work" };
const pending: PendingOperation = { id: "unknown", workspace, action: "commit", paths: ["one.txt"], startedAt: 1 };
function client(): Api {
  return {
    load: vi.fn(), save: vi.fn(), remove: vi.fn(), configure: vi.fn(), inspect: vi.fn(), status: vi.fn(), diff: vi.fn(), history: vi.fn(), chooseDirectory: vi.fn(),
    pendingOperations: vi.fn().mockResolvedValue([]), inspectPending: vi.fn().mockResolvedValue({ operation: pending, changes: [], history: [] }), acknowledgePending: vi.fn().mockResolvedValue(undefined),
    previewWrite: vi.fn().mockImplementation((_id: string, request: WriteRequest) => Promise.resolve({ token: "preview", workspace, action: request.action, paths: request.paths, warnings: ["選択対象を確認してください"] })),
    executeWrite: vi.fn().mockResolvedValue({ output: "Committed revision 2.", revision: 2 }),
  };
}

describe("書込みの確認", () => {
  it("対象とメッセージを確認してから一回限りのトークンで実行する", async () => {
    const api = client();
    render(<WritePanel workspace={workspace} paths={["one.txt"]} client={api} blocked={false} onBusyChange={() => {}} onFinished={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: "コミット" }));
    await userEvent.type(screen.getByLabelText("コミットメッセージ"), "変更の記録");
    await userEvent.click(screen.getByRole("button", { name: "対象を確認" }));
    expect(await screen.findByText("/srv/work")).toBeTruthy();
    expect(screen.getByText("one.txt")).toBeTruthy();
    expect((screen.getByRole("button", { name: "実行する" }) as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(screen.getByLabelText("対象と影響を確認しました"));
    await userEvent.click(screen.getByRole("button", { name: "実行する" }));
    expect(await screen.findByText(/リビジョン r2/)).toBeTruthy();
    expect(api.previewWrite).toHaveBeenCalledWith("one", { action: "commit", paths: ["one.txt"], message: "変更の記録" });
    expect(api.executeWrite).toHaveBeenCalledExactlyOnceWith("one", "preview");
  });

  it("成否不明の場合は同じ操作の再実行を提供しない", async () => {
    const api = client();
    api.executeWrite = vi.fn().mockRejectedValue({ kind: "outcomeUnknown", message: "成否を確認してください" });
    render(<WritePanel workspace={workspace} paths={["one.txt"]} client={api} blocked={false} onBusyChange={() => {}} onFinished={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: "変更を破棄" }));
    await userEvent.click(screen.getByRole("button", { name: "対象を確認" }));
    await userEvent.click(await screen.findByLabelText("対象と影響を確認しました"));
    await userEvent.click(screen.getByRole("button", { name: "実行する" }));
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", expect.stringContaining("成否を確認"));
    expect(screen.queryByRole("button", { name: "実行する" })).toBeNull();
    expect(api.executeWrite).toHaveBeenCalledTimes(1);
    expect((screen.getByRole("button", { name: "コミット" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("未完了操作は成否と停止の明示確認なしに解除できない", async () => {
    const api = client();
    render(<RecoveryPanel operations={[pending]} client={api} onChanged={() => {}} onBusyChange={() => {}} />);
    expect((screen.getByRole("button", { name: "書込み制限を解除" }) as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "状態・最新履歴を確認" }));
    expect(await screen.findByText("現在の変更: 0件")).toBeTruthy();
    await userEvent.click(screen.getByLabelText("外部で処理の停止と成否を確認しました"));
    await userEvent.click(screen.getByRole("button", { name: "書込み制限を解除" }));
    expect(api.acknowledgePending).toHaveBeenCalledExactlyOnceWith("unknown", true);
  });

  it("未完了操作がある作業コピーでは書込みを開始できない", () => {
    render(<WritePanel workspace={workspace} paths={["one.txt"]} client={client()} blocked onBusyChange={() => {}} onFinished={() => {}} />);
    expect((screen.getByRole("button", { name: "コミット" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "更新" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
