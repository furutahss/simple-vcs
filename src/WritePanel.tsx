import { useEffect, useState } from "react";
import type { Api, PendingOperation, RecoveryInfo, Workspace, WriteAction, WritePreview, WriteResult } from "./types";

const actions: Record<WriteAction, string> = {
  add: "追加予定にする", delete: "削除予定にする", commit: "コミット", update: "更新",
  revert: "変更を破棄", cleanup: "cleanup", resolve: "解決済みにする",
};
function message(error: unknown): string { return error && typeof error === "object" && "message" in error ? String(error.message) : String(error); }
function outcomeUnknown(error: unknown): boolean { return !!error && typeof error === "object" && "kind" in error && error.kind === "outcomeUnknown"; }
function target(workspace: Workspace): string { return workspace.target.kind === "local" ? "ローカル" : `SSH: ${workspace.target.host}`; }

export function WritePanel({ workspace, paths, client, blocked, onBusyChange, onFinished }: {
  workspace: Workspace; paths: string[]; client: Api; blocked: boolean;
  onBusyChange: (busy: boolean) => void; onFinished: () => void;
}) {
  const [draft, setDraft] = useState<{ action: WriteAction; paths: string[] } | null>(null);
  const [commitMessage, setCommitMessage] = useState("");
  const [preview, setPreview] = useState<WritePreview | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [error, setError] = useState("");
  const [result, setResult] = useState<WriteResult | null>(null);
  useEffect(() => { onBusyChange(busy); return () => onBusyChange(false); }, [busy, onBusyChange]);
  useEffect(() => { if (!blocked) setUncertain(false); }, [blocked]);

  function open(action: WriteAction) {
    setDraft({ action, paths: ["update", "cleanup"].includes(action) ? [] : [...paths] });
    setCommitMessage(""); setPreview(null); setConfirmed(false); setError(""); setResult(null);
  }
  async function prepare() {
    if (!draft) return;
    setBusy(true); setError(""); setConfirmed(false);
    try { setPreview(await client.previewWrite(workspace.id, { ...draft, message: commitMessage })); }
    catch (reason) { setError(message(reason)); setPreview(null); }
    finally { setBusy(false); }
  }
  async function execute() {
    if (!preview || !confirmed || busy || blocked || uncertain) return;
    setBusy(true); setError("");
    try {
      setResult(await client.executeWrite(preview.workspace.id, preview.token));
      setDraft(null); setPreview(null); onFinished();
    } catch (reason) {
      setError(message(reason)); setPreview(null); setConfirmed(false);
      if (outcomeUnknown(reason)) { setUncertain(true); setDraft(null); onFinished(); }
    } finally { setBusy(false); }
  }
  return <section className="write-tools" aria-label="SVN操作">
    <div className="write-toolbar"><span>{paths.length}件選択</span>
      {(Object.keys(actions) as WriteAction[]).map(action => <button key={action} className={action === "commit" ? "primary" : action === "revert" || action === "delete" ? "destructive" : ""}
        disabled={blocked || busy || uncertain || (!["update", "cleanup"].includes(action) && paths.length === 0)} onClick={() => open(action)}>{actions[action]}</button>)}
    </div>
    {draft && <div className="write-confirmation"><h2>{actions[draft.action]}の確認</h2>
      {!preview && <><p>{["update", "cleanup"].includes(draft.action) ? "作業コピー全体が対象です。" : `選択した${draft.paths.length}件を対象にします。`}</p>
        {draft.action === "commit" && <label>コミットメッセージ<textarea autoFocus value={commitMessage} maxLength={65536} disabled={busy} onChange={event => setCommitMessage(event.target.value)} rows={4} /></label>}
        <div className="form-actions"><button className="primary" disabled={blocked || busy || (draft.action === "commit" && !commitMessage.trim())} onClick={() => void prepare()}>{busy ? "確認中…" : "対象を確認"}</button><button disabled={busy} onClick={() => setDraft(null)}>閉じる</button></div>
      </>}
      {preview && <><div className="write-destination"><strong>{target(preview.workspace)}</strong><code>{preview.workspace.path}</code></div>
        {preview.warnings.map(warning => <p className="warning" key={warning}>{warning}</p>)}
        <ul className="write-targets">{preview.paths.map(path => <li key={path}><code>{path}</code></li>)}</ul>
        {draft.action === "commit" && <pre className="confirmed-message">{commitMessage}</pre>}
        <label className="checkbox-label"><input type="checkbox" checked={confirmed} disabled={busy} onChange={event => setConfirmed(event.target.checked)} />対象と影響を確認しました</label>
        <div className="form-actions"><button className="primary" disabled={!confirmed || busy || blocked} onClick={() => void execute()}>{busy ? "実行中・結果確認中…" : "実行する"}</button><button disabled={busy} onClick={() => { setPreview(null); setConfirmed(false); }}>内容を変更</button><button disabled={busy} onClick={() => { setDraft(null); setPreview(null); }}>閉じる</button></div>
        {busy && <p>処理を再送せず待機しています。アプリや接続を終了すると成否不明になる場合があります。</p>}
      </>}
    </div>}
    {error && <p role="alert" className="error">{error}</p>}
    {result && <div className="write-result" role="status"><strong>{result.revision ? `コミット完了 · リビジョン r${result.revision}` : "操作が完了しました。最新状態を再取得しています。"}</strong><pre>{result.output}</pre></div>}
  </section>;
}

export function RecoveryPanel({ operations, client, onChanged, onBusyChange }: {
  operations: PendingOperation[]; client: Api; onChanged: () => void; onBusyChange: (busy: boolean) => void;
}) {
  const [report, setReport] = useState<RecoveryInfo | null>(null);
  const [checked, setChecked] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { onBusyChange(busy); return () => onBusyChange(false); }, [busy, onBusyChange]);
  async function inspect(id: string) {
    setBusy(true); setReport(null); setError("");
    try { setReport(await client.inspectPending(id)); } catch (reason) { setError(message(reason)); }
    finally { setBusy(false); }
  }
  async function acknowledge(id: string) {
    if (!checked.includes(id)) return;
    setBusy(true); setError("");
    try { await client.acknowledgePending(id, true); setReport(null); onChanged(); }
    catch (reason) { setError(message(reason)); }
    finally { setBusy(false); }
  }
  if (!operations.length) return null;
  return <section className="recovery-panel" aria-label="成否不明の操作"><h2>成否の確認が必要です</h2><p>自動再実行はしません。状態・最新履歴を確認し、実行先で処理が停止していることと操作の成否を確認してください。表示結果だけでは処理の停止を保証できません。</p>
    {operations.map(operation => <article key={operation.id}><strong>{operation.workspace.name} · {actions[operation.action]}</strong><p>{target(operation.workspace)} · <code>{operation.workspace.path}</code></p><small>開始 {new Date(operation.startedAt * 1000).toLocaleString("ja-JP")}</small>
      <ul className="write-targets">{operation.paths.map(path => <li key={path}><code>{path}</code></li>)}</ul>
      <button disabled={busy} onClick={() => void inspect(operation.id)}>状態・最新履歴を確認</button>
      {report?.operation.id === operation.id && <div className="recovery-report"><p>現在の変更: {report.changes.length}件</p><ul>{report.changes.map(change => <li key={change.path}>{change.status} · {change.path}</li>)}</ul><h3>リポジトリの最新履歴</h3><ul>{report.history.map(revision => <li key={revision.revision}>r{revision.revision} · {revision.author} · {revision.message}</li>)}</ul></div>}
      <label className="checkbox-label"><input type="checkbox" disabled={busy} checked={checked.includes(operation.id)} onChange={event => setChecked(previous => event.target.checked ? [...previous, operation.id] : previous.filter(id => id !== operation.id))} />外部で処理の停止と成否を確認しました</label>
      <button disabled={busy || !checked.includes(operation.id)} onClick={() => void acknowledge(operation.id)}>書込み制限を解除</button>
    </article>)}{error && <p className="error" role="alert">{error}</p>}
  </section>;
}
