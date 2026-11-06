import { useEffect, useRef, useState, type FormEvent } from "react";
import type { Api, Change, Info, Revision, Settings, Workspace, PendingOperation, GitChange, GitCommit, GitInfo, GitBranch } from "./types";

import { WritePanel, RecoveryPanel } from "./WritePanel";

const labels: Record<string, string> = { modified: "変更", added: "追加予定", deleted: "削除予定", unversioned: "未追跡", missing: "欠損", replaced: "置換", conflicted: "競合", external: "外部作業コピー", ignored: "無視", normal: "通常", obstructed: "障害", incomplete: "不完全", none: "なし" };
function message(error: unknown): string { return error && typeof error === "object" && "message" in error ? String(error.message) : String(error); }
function destination(w: Workspace) { return w.target.kind === "local" ? "ローカル" : w.target.host; }
function date(value: string) { return value ? new Date(value).toLocaleString("ja-JP") : "日時なし"; }

export default function App({ client }: { client: Api }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [id, setId] = useState("");
  const [tab, setTab] = useState<"changes" | "history">("changes");
  const [info, setInfo] = useState<Info | null>(null);
  const [changes, setChanges] = useState<Change[] | null>(null);
  const [revisions, setRevisions] = useState<Revision[] | null>(null);
  const [revision, setRevision] = useState<Revision | null>(null);
  const [selectedPath, setSelectedPath] = useState("");
  const [diff, setDiff] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [detailError, setDetailError] = useState("");
  const [busy, setBusy] = useState(false);
  const [detailBusy, setDetailBusy] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [updated, setUpdated] = useState("");
  const [logs, setLogs] = useState<string[]>([]);
  const [editing, setEditing] = useState<Workspace | "new" | null>(null);
  const [toolsOpen, setToolsOpen] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [paging, setPaging] = useState(false);
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [includeUnchanged, setIncludeUnchanged] = useState(false);
  const [writeBusy, setWriteBusy] = useState(false);
  const [pending, setPending] = useState<PendingOperation[]>([]);
  const [pendingReady, setPendingReady] = useState(false);
  const [pendingError, setPendingError] = useState("");
  const generation = useRef(0);
  const detailGeneration = useRef(0);
  const workspace = settings?.workspaces.find(item => item.id === id);

  function log(text: string) { setLogs(previous => [`${new Date().toLocaleTimeString("ja-JP")} ${text}`, ...previous].slice(0, 20)); }
  function invalidate() { generation.current++; detailGeneration.current++; }
  function select(next: string) { if (next !== id) { invalidate(); setSelectedPaths([]); setId(next); } setEditing(null); setToolsOpen(false); }
  function reload() { invalidate(); setRefresh(previous => previous + 1); }
  async function refreshPending() {
    setPendingReady(false);
    try { setPending(await client.pendingOperations()); setPendingError(""); }
    catch (reason) { setPendingError(message(reason)); }
    finally { setPendingReady(true); }
  }
  function afterWrite() { setSelectedPaths([]); void refreshPending(); reload(); }

  useEffect(() => {
    let active = true;
    client.pendingOperations().then(value => { if (active) { setPending(value); setPendingError(""); setPendingReady(true); } })
      .catch(reason => { if (active) { setPendingError(message(reason)); setPendingReady(true); } });
    client.load().then(value => { if (active) { setSettings(value); setId(value.workspaces[0]?.id ?? ""); } })
      .catch(reason => { if (active) setError(message(reason)); });
    return () => { active = false; invalidate(); };
  }, [client]);

  useEffect(() => {
    const token = ++generation.current;
    detailGeneration.current++;
    setChanges(null); setRevisions(null); setInfo(null); setRevision(null); setSelectedPath("");
    setDiff(null); setError(""); setDetailError(""); setUpdated(""); setDetailBusy(false); setPaging(false);
    if (!id) { setBusy(false); return; }
    setBusy(true);
    if (workspace?.vcs === "git") { setBusy(false); return () => { if (token === generation.current) invalidate(); }; }
    const fetch = tab === "changes" ? (includeUnchanged ? client.status(id, true) : client.status(id)) : client.history(id);
    Promise.all([client.inspect(id), fetch]).then(([metadata, values]) => {
      if (token !== generation.current) return;
      setInfo(metadata);
      if (tab === "changes") { setChanges(values as Change[]); setSelectedPaths(previous => previous.filter(path => (values as Change[]).some(change => change.path === path))); }
      else { setRevisions(values as Revision[]); setHasMore(values.length === 100); }
      setUpdated(new Date().toLocaleTimeString("ja-JP"));
      log(`${tab === "changes" ? "変更一覧" : "履歴"}を取得しました`);
    }).catch(reason => {
      if (token === generation.current) { setError(message(reason)); log("取得に失敗しました"); }
    }).finally(() => { if (token === generation.current) setBusy(false); });
    return () => { if (token === generation.current) invalidate(); };
  }, [id, tab, refresh, includeUnchanged, client, workspace?.vcs]);

  useEffect(() => {
    const onFocus = () => { if (id && !editing && !toolsOpen && !writeBusy) reload(); };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [id, editing, toolsOpen, writeBusy]);

  async function showDiff(path: string, revisionNumber?: number) {
    const context = generation.current;
    const token = ++detailGeneration.current;
    setSelectedPath(path); setDiff(null); setDetailError(""); setDetailBusy(true);
    try {
      const value = revisionNumber === undefined ? await client.diff(id, path) : await client.diff(id, path, revisionNumber);
      if (token === detailGeneration.current && context === generation.current) setDiff(value);
    } catch (reason) {
      if (token === detailGeneration.current && context === generation.current) setDetailError(message(reason));
    } finally {
      if (token === detailGeneration.current && context === generation.current) setDetailBusy(false);
    }
  }

  async function more() {
    if (!revisions?.length || paging) return;
    const context = generation.current;
    setPaging(true);
    try {
      const last = revisions[revisions.length - 1].revision;
      const values = last > 1 ? await client.history(id, last - 1) : [];
      if (context !== generation.current) return;
      setRevisions(previous => [...(previous ?? []), ...values]); setHasMore(values.length === 100);
    } catch (reason) { if (context === generation.current) setError(message(reason)); }
    finally { if (context === generation.current) setPaging(false); }
  }

  if (workspace?.vcs === "git") return <GitWorkspace key={workspace.id} workspace={workspace} settings={settings!} client={client} operations={pending} pendingError={pendingError} refreshKey={refresh} blocked={!pendingReady || !!pendingError || pending.length > 0 || writeBusy} onBusy={setWriteBusy} onRefresh={afterWrite} onSelect={select} editing={editing} onEdit={setEditing} onSaved={(value, selected) => { setSettings(value); setEditing(null); select(selected); afterWrite(); }} />;

  return <div className="app">
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark" aria-hidden="true">⑂</span><div>Simple VCS<small>作業を、見通しよく。</small></div></div>
      <div className="sidebar-heading">作業コピー <span>{settings?.workspaces.length ?? 0}</span></div>
      <nav aria-label="作業コピー" className="workspaces">
        {settings?.workspaces.map(item => <button key={item.id} className={id === item.id ? "workspace selected" : "workspace"} aria-current={id === item.id ? "page" : undefined} disabled={writeBusy} onClick={() => select(item.id)}>
          <span aria-hidden="true">{item.target.kind === "local" ? "▣" : "⇄"}</span>
          <span><strong>{item.name}</strong><small>{item.target.kind === "local" ? "ローカル" : `SSH · ${item.target.host}`}</small></span><span className="vcs-badge">{item.vcs.toUpperCase()}</span>
        </button>)}
      </nav>
      <button className="add-workspace" aria-label="作業コピーを登録" disabled={!settings || writeBusy} onClick={() => { setEditing("new"); setToolsOpen(false); }}>＋ 作業コピーを登録</button>
      <div className="sidebar-bottom"><p>VCS ワークスペース<span>SVN / Git · ローカル / SSH</span></p><button onClick={() => setToolsOpen(value => !value)} disabled={!settings || writeBusy}>CLI設定</button></div>
    </aside>
    <main>
      <header className="topbar"><div className="breadcrumb">作業コピー <span>/</span> <strong>{workspace?.name ?? "未選択"}</strong></div><span className="read-only">{writeBusy ? "操作中" : "SVN"}</span></header>
      {toolsOpen && settings && <Tools settings={settings} client={client} onSaved={value => { setSettings(value); setToolsOpen(false); reload(); }} onClose={() => setToolsOpen(false)} />}
      {editing && settings && <Registration key={editing === "new" ? "new" : editing.id} initial={editing === "new" ? null : editing} client={client} onSaved={(value, selected) => { setSettings(value); setEditing(null); select(selected); reload(); }} onClose={() => setEditing(null)} />}
      <section className="workspace-heading"><div><div className="eyebrow">SUBVERSION WORKSPACE</div><h1>{workspace?.name ?? "作業コピーを開く"}</h1><div className="target-line">{workspace && <><span className="target-badge">{destination(workspace)}</span><code>{workspace.path}</code></>}</div></div><div className="actions"><button disabled={!workspace || busy || writeBusy} onClick={() => workspace && setEditing(workspace)}>登録内容</button><button className="primary" disabled={!workspace || busy || writeBusy} onClick={reload}>{busy ? "取得中…" : "↻ 再取得"}</button></div></section>
      {info && <div className="repo-meta"><span title="作業コピールートのリビジョンです。コミット後もファイルごとに異なる場合があります。">作業コピー r{info.revision}</span><span>SVN {info.svnVersion}</span><code title={info.url}>{info.url}</code><span className="updated">取得 {updated}</span></div>}
      {info?.warnings.map(warning => <p className="warning" key={warning}>{warning}</p>)}
      {error && <div role="alert" className="error"><strong>取得できませんでした</strong><p>{error}</p></div>}
      {pendingError && <div className="error" role="alert">操作記録を確認できないため書込みを停止しています。{pendingError}</div>}
      <RecoveryPanel operations={pending} client={client} onChanged={afterWrite} onBusyChange={setWriteBusy} />
      {workspace && tab === "changes" && <WritePanel key={workspace.id} workspace={workspace} paths={selectedPaths} client={client}
        blocked={!pendingReady || !!pendingError || pending.length > 0 || !!error || busy || writeBusy || !info || info.warnings.length > 0}
        onBusyChange={setWriteBusy} onFinished={afterWrite} />}
      <div className="tabs" role="tablist" aria-label="表示内容" onKeyDown={event => {
        if (!workspace || writeBusy || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        const next = event.key === "Home" ? "changes" : event.key === "End" ? "history" : tab === "changes" ? "history" : "changes";
        if (next !== tab) { invalidate(); setTab(next); }
        event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next === "changes" ? 0 : 1].focus();
      }}>
        <button id="changes-tab" role="tab" aria-controls="workspace-panel" tabIndex={tab === "changes" ? 0 : -1} aria-selected={tab === "changes"} onClick={() => { if (tab !== "changes") { invalidate(); setTab("changes"); } }} disabled={!workspace || writeBusy}>変更 {changes && <span>{changes.length}</span>}</button>
        <button id="history-tab" role="tab" aria-controls="workspace-panel" tabIndex={tab === "history" ? 0 : -1} aria-selected={tab === "history"} onClick={() => { if (tab !== "history") { invalidate(); setTab("history"); } }} disabled={!workspace || writeBusy}>履歴</button>
      </div>
      <section id="workspace-panel" role="tabpanel" aria-labelledby={`${tab}-tab`} className="panes" aria-busy={busy}>
        <div className="list-pane"><div className="pane-heading"><span>{tab === "changes" ? (includeUnchanged ? "作業コピーのファイル" : "変更ファイル") : "リビジョン"}</span><small>{tab === "changes" ? "BASEとの比較" : "新しい順"}</small></div>
          {tab === "changes" && workspace && <label className="select-all checkbox-label"><input type="checkbox" checked={includeUnchanged} disabled={writeBusy} onChange={event => { invalidate(); setIncludeUnchanged(event.target.checked); }} />通常ファイルも表示</label>}
          {busy && <p className="empty" role="status">{workspace?.target.kind === "ssh" ? "SSH先から取得しています…" : "取得しています…"}</p>}
          {!workspace && !error && <p className="empty">作業コピーを登録すると、変更・差分・履歴を確認できます。</p>}
          {changes?.length === 0 && !error && <p className="empty">変更はありません</p>}
          {changes && changes.length > 0 && <label className="select-all checkbox-label"><input type="checkbox" disabled={writeBusy} checked={selectedPaths.length === changes.length} onChange={event => setSelectedPaths(event.target.checked ? changes.map(change => change.path) : [])} />すべて選択</label>}
          {changes?.map(change => <div className="selectable-change" key={change.path}><input type="checkbox" aria-label={`${change.path} を選択`} disabled={writeBusy} checked={selectedPaths.includes(change.path)} onChange={event => setSelectedPaths(previous => event.target.checked ? [...previous, change.path] : previous.filter(path => path !== change.path))} /><button className={`change-row ${selectedPath === change.path ? "active" : ""}`} onClick={() => void showDiff(change.path)}><span className={`status-label status-${change.status}`}>{labels[change.status] ?? change.status}</span><span className="file-path">{change.path}<small>{[change.properties !== "none" && change.properties !== "normal" ? `プロパティ: ${labels[change.properties] ?? change.properties}` : "", change.treeConflicted ? "ツリー競合" : "", change.locked ? "作業コピーロック" : "", change.copied ? "コピー" : "", change.switched ? "切替パス" : ""].filter(Boolean).join(" · ")}</small></span></button></div>)}
          {revisions?.length === 0 && !error && <p className="empty">履歴はありません</p>}
          {revisions?.map(item => <button key={item.revision} className={`revision-row ${revision?.revision === item.revision ? "active" : ""}`} onClick={() => { detailGeneration.current++; setRevision(item); setSelectedPath(""); setDiff(null); setDetailError(""); setDetailBusy(false); }}><span className="revision-number">r{item.revision}</span><strong>{item.message.split("\n")[0] || "メッセージなし"}</strong><small>{item.author || "作者なし"} · {date(item.date)}</small></button>)}
          {hasMore && revisions && <button className="load-more" disabled={paging} onClick={() => void more()}>{paging ? "取得中…" : "次の100件"}</button>}
        </div>
        <div className="detail-pane"><div className="pane-heading"><span>{selectedPath || (revision ? `r${revision.revision} の詳細` : "差分プレビュー")}</span><small>{revision ? "リビジョンの変更" : "作業コピー ↔ BASE"}</small></div>
          {revision && <div className="revision-detail"><div><strong>r{revision.revision}</strong> · {revision.author} · {date(revision.date)}</div><p className="commit-message">{revision.message}</p><div className="history-paths">{revision.paths.map(path => <button key={path.path} onClick={() => void showDiff(path.path, revision.revision)}><span>{path.action}</span><code>{path.path}</code></button>)}</div></div>}
          {detailBusy && <p className="empty" role="status">差分を取得しています…</p>}{detailError && <p className="error" role="alert">{detailError}</p>}
          {diff !== null && !detailBusy && <div className="diff" aria-label="差分">{diff ? diff.split("\n").map((line, index) => <div key={index} className={line.startsWith("+") ? "added" : line.startsWith("-") ? "removed" : line.startsWith("@@") ? "hunk" : ""}><span className="line-number" aria-hidden="true">{index + 1}</span><code>{line || " "}</code></div>) : <p className="empty">表示できるテキスト差分はありません</p>}</div>}
          {!selectedPath && !revision && <div className="detail-empty"><span aria-hidden="true">≋</span><h2>{tab === "changes" ? "変更を選んで、内容を確認" : "リビジョンを選んで、履歴を確認"}</h2><p>{tab === "changes" ? "ファイルを選ぶと、ここに差分が表示されます。" : "メッセージと変更パスから差分を追えます。"}</p></div>}
        </div>
      </section>
      <footer className="statusbar"><span className={`status-dot ${error ? "failed" : ""}`} aria-hidden="true"/><span>{busy ? "読取り中" : error ? "取得失敗" : updated ? "取得済み" : "待機中"}</span><span>書込み前に対象と影響を確認します</span><details><summary>操作ログ</summary><ul>{logs.map((line, index) => <li key={index}>{line}</li>)}</ul></details></footer>
    </main>
  </div>;
}

function Registration({ initial, client, onSaved, onClose }: { initial: Workspace | null; client: Api; onSaved: (settings: Settings, id: string) => void; onClose: () => void }) {
  const [name, setName] = useState(initial?.name ?? "");
  const [kind, setKind] = useState(initial?.target.kind ?? "local");
  const [host, setHost] = useState(initial?.target.kind === "ssh" ? initial.target.host : "");
  const [path, setPath] = useState(initial?.path ?? "");
  const [vcs, setVcs] = useState(initial?.vcs ?? "svn");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault(); setBusy(true); setError("");
    const workspace: Workspace = { id: initial?.id ?? crypto.randomUUID(), name: name.trim(), path, vcs, target: kind === "local" ? { kind: "local" } : { kind: "ssh", host: host.trim() } };
    try { onSaved(await client.save(workspace), workspace.id); } catch (reason) { setError(message(reason)); } finally { setBusy(false); }
  }
  async function remove() {
    if (!initial) return;
    setBusy(true);
    try { const value = await client.remove(initial.id); onSaved(value, value.workspaces[0]?.id ?? ""); } catch (reason) { setError(message(reason)); } finally { setBusy(false); }
  }
  return <section className="editor" aria-label="作業コピー登録"><h2>{initial ? "登録内容を編集" : "作業コピーを登録"}</h2><p>既存のSVN作業コピーを指定してください。ファイルの同期・変更は行いません。</p><form onSubmit={event => void submit(event)}><div className="form-grid">
    <label>表示名<input autoFocus required value={name} onChange={event => setName(event.target.value)} /></label><label>バージョン管理<select value={vcs} onChange={event => setVcs(event.target.value as "svn" | "git")}><option value="svn">SVN</option><option value="git">Git</option></select></label><label>実行先<select value={kind} onChange={event => { setKind(event.target.value as "local" | "ssh"); setPath(""); }}><option value="local">ローカル</option><option value="ssh">SSH先（Linux）</option></select></label>
    {kind === "ssh" && <label>SSH Host別名<input required value={host} onChange={event => setHost(event.target.value)} placeholder="例: team-server" /></label>}
    <label className="path-field">作業コピーの絶対パス<div className="input-with-button"><input required value={path} onChange={event => setPath(event.target.value)} placeholder={kind === "local" ? "C:\\Projects\\example" : "/srv/projects/example"} />{kind === "local" && <button type="button" disabled={busy} onClick={() => { void client.chooseDirectory().then(value => { if (value) setPath(value); }).catch(reason => setError(message(reason))); }}>参照</button>}</div></label>
    </div>{kind === "ssh" && <p className="hint">WindowsのSSH configと鍵・agentを利用します。接続先にはSVNとUTF-8ロケールが必要です。</p>}{error && <p role="alert" className="error">{error}</p>}
    <div className="form-actions"><button className="primary" disabled={busy}>{busy ? "処理中…" : "保存して開く"}</button><button type="button" disabled={busy} onClick={onClose}>キャンセル</button>{initial && <button className="danger" type="button" disabled={busy} onClick={() => setRemoving(true)}>登録を解除</button>}</div>{removing && <div className="warning">一覧から登録だけを解除します。実ファイルは削除しません。<button type="button" disabled={busy} onClick={() => void remove()}>解除する</button><button type="button" onClick={() => setRemoving(false)}>戻る</button></div>}</form></section>;
}

function Tools({ settings, client, onSaved, onClose }: { settings: Settings; client: Api; onSaved: (settings: Settings) => void; onClose: () => void }) {
  const [svn, setSvn] = useState(settings.svn);
  const [ssh, setSsh] = useState(settings.ssh);
  const [git, setGit] = useState(settings.git);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault(); setBusy(true);
    try { onSaved(await client.configure(svn, git, ssh)); } catch (reason) { setError(message(reason)); } finally { setBusy(false); }
  }
  return <section className="editor" aria-label="CLI設定"><h2>CLI設定</h2><p>引数は入力せず、実行ファイル名または絶対パスを指定してください。SSH先のCLIは接続先のPATHから実行します。</p><form onSubmit={event => void submit(event)}><div className="form-grid"><label>ローカルSVN<input required value={svn} onChange={event => setSvn(event.target.value)} /></label><label>ローカルGit<input required value={git} onChange={event => setGit(event.target.value)} /></label><label>OpenSSH<input required value={ssh} onChange={event => setSsh(event.target.value)} /></label></div>{error && <p className="error" role="alert">{error}</p>}<div className="form-actions"><button className="primary" disabled={busy}>保存</button><button type="button" disabled={busy} onClick={onClose}>閉じる</button></div></form></section>;
}

function GitWorkspace({ workspace, settings, client, operations, pendingError, refreshKey, blocked, onBusy, onRefresh, onSelect, editing, onEdit, onSaved }: { workspace: Workspace; settings: Settings; client: Api; operations: PendingOperation[]; pendingError: string; refreshKey: number; blocked: boolean; onBusy: (busy: boolean) => void; onRefresh: () => void; onSelect: (id: string) => void; editing: Workspace | "new" | null; onEdit: (value: Workspace | "new" | null) => void; onSaved: (settings: Settings, id: string) => void }) {
  const [info, setInfo] = useState<GitInfo | null>(null);
  const [changes, setChanges] = useState<GitChange[]>([]);
  const [branches, setBranches] = useState<GitBranch[]>([]);
  const [history, setHistory] = useState<GitCommit[]>([]);
  const [tab, setTab] = useState<"changes" | "history">("changes");
  const [selected, setSelected] = useState("");
  const [diff, setDiff] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [selection, setSelection] = useState<string[]>([]);
  const [revision, setRevision] = useState<GitCommit | null>(null);
  const [commitDraft, setCommitDraft] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");
  const [commitConfirmed, setCommitConfirmed] = useState(false);
  const [skip, setSkip] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(false);

  async function reload() {
    setBusy(true); setError(""); setCommitConfirmed(false);
    try {
      const [metadata, status, refs, commits] = await Promise.all([client.gitInfo(workspace.id), client.gitStatus(workspace.id), client.gitBranches(workspace.id), client.gitHistory(workspace.id, 0)]);
      setInfo(metadata); setChanges(status); setBranches(refs); setHistory(commits); setHasMore(commits.length === 100); setSkip(0); setSelection(old => old.filter(path => status.some(item => item.path === path)));
    } catch (reason) { setError(message(reason)); }
    finally { setBusy(false); }
  }
  useEffect(() => { void reload(); }, [workspace.id, refreshKey]);
  useEffect(() => { onBusy(busy); return () => onBusy(false); }, [busy, onBusy]);

  async function write(action: "stage" | "unstage" | "discard" | "commit" | "branch" | "switch" | "fetch" | "pull" | "push", paths: string[] = [], input = "") {
    if (busy || blocked) return;
    if (action === "discard" && !window.confirm(`未ステージの変更を破棄します。ステージ済み変更と未追跡ファイルは対象外です。\n\n${paths.join("\n")}\n\n続けますか？`)) return;
    if (["pull", "push"].includes(action)) {
      if (info?.remote) { const [remote, ...branch] = info.remote.split("/"); input = `${remote} ${branch.join("/")}`; }
      else { const target = window.prompt("remote名とbranch名を空白区切りで入力してください"); if (!target?.trim()) return; input = target.trim(); }
    }
    if (["branch", "switch", "pull", "push"].includes(action) && !window.confirm(`${action}${input ? ` ${input}` : ""} を ${workspace.name} (${workspace.path}) で実行します。続けますか？`)) return;
    setBusy(true); setError("");
    try { await client.gitWrite(workspace.id, action, paths, input); setSelected(""); setRevision(null); setDiff(""); onRefresh(); }
    catch (reason) { setError(message(reason)); onRefresh(); }
    finally { setBusy(false); }
  }
  async function showDiff(path: string, staged: boolean) {
    setSelected(path); setRevision(null); setDiff("");
    try { setDiff(await client.gitDiff(workspace.id, path, staged)); } catch (reason) { setError(message(reason)); }
  }
  function commit() {
    if (!staged.length) return;
    setCommitMessage(""); setCommitConfirmed(false); setCommitDraft(true);
  }
  async function executeCommit() {
    if (!commitConfirmed || !commitMessage.trim()) return;
    setCommitDraft(false);
    await write("commit", [], commitMessage);
  }
  async function pageMore() {
    const next = skip + history.length;
    try { const result = await client.gitHistory(workspace.id, next); setHistory(previous => [...previous, ...result]); setHasMore(result.length === 100); setSkip(next); }
    catch (reason) { setError(message(reason)); }
  }
  async function showCommit(commit: GitCommit) {
    setRevision(commit); setSelected(""); setDiff("");
    try { setDiff(await client.gitCommitDiff(workspace.id, commit.hash)); }
    catch (reason) { setError(message(reason)); }
  }
  const staged = changes.filter(change => change.index !== ".");
  const unstaged = changes.filter(change => change.worktree !== "." || change.untracked || change.conflicted);
  const available = blocked || busy || !info || !!info.operation || !!info.warnings.length;
  return <div className="app"><aside className="sidebar"><div className="brand"><span className="brand-mark" aria-hidden="true">⑂</span><div>Simple VCS<small>作業を、見通しよく。</small></div></div><div className="sidebar-heading">作業コピー <span>{settings.workspaces.length}</span></div><nav aria-label="作業コピー" className="workspaces">{settings.workspaces.map(item => <button key={item.id} className={workspace.id === item.id ? "workspace selected" : "workspace"} aria-current={workspace.id === item.id ? "page" : undefined} disabled={busy || blocked} onClick={() => onSelect(item.id)}><span aria-hidden="true">{item.target.kind === "local" ? "▣" : "⇄"}</span><span><strong>{item.name}</strong><small>{item.target.kind === "local" ? "ローカル" : `SSH · ${item.target.host}`}</small></span><span className="vcs-badge">{item.vcs.toUpperCase()}</span></button>)}</nav><button className="add-workspace" disabled={busy || blocked} onClick={() => onEdit("new")}>＋ 作業コピーを登録</button><div className="sidebar-bottom"><p>SVN / Git<span>ローカル / SSH</span></p><button disabled={busy || blocked} onClick={() => setToolsOpen(value => !value)}>CLI設定</button></div></aside><main className="git-view">
    {toolsOpen && <Tools settings={settings} client={client} onSaved={value => { setToolsOpen(false); onSaved(value, workspace.id); }} onClose={() => setToolsOpen(false)} />}
    {editing && <Registration key={editing === "new" ? "new" : editing.id} initial={editing === "new" ? null : editing} client={client} onSaved={onSaved} onClose={() => onEdit(null)} />}
    <header className="git-heading"><div><div className="eyebrow">GIT WORKSPACE</div><h1>{workspace.name}</h1><div className="target-line"><span className="target-badge">{destination(workspace)}</span><code>{workspace.path}</code></div></div><div className="actions"><button onClick={() => onEdit(workspace)} disabled={busy || blocked}>登録内容</button><button onClick={() => void reload()} disabled={busy}>↻ 再取得</button></div></header>
    {info && <div className="repo-meta"><span>branch: {info.branch || "未作成"}</span><span>Git {info.version}</span><span>{info.remote || "追跡先未設定"}</span>{info.head && <code title={info.head}>{info.head.slice(0, 12)}</code>}</div>}
    {info?.warnings.map(warning => <p className="warning" key={warning}>{warning}</p>)}
    {info?.operation && <p className="warning">Gitの {info.operation} が進行中です。外部で完了または中止してください。</p>}
    {info?.detached && <p className="warning">detached HEADです。ブランチの切替・作成とcommit/pushを停止しています。</p>}
    {operations.length > 0 && <RecoveryPanel operations={operations} client={client} onChanged={onRefresh} onBusyChange={onBusy} />}
    {(error || pendingError) && <div className="error" role="alert">{error || pendingError}</div>}
    <div className="git-toolbar" aria-label="Git操作">
      <button disabled={available || commitDraft || !selection.some(path => unstaged.some(item => item.path === path && !item.conflicted && !item.submodule))} onClick={() => void write("stage", selection.filter(path => unstaged.some(item => item.path === path && !item.conflicted && !item.submodule)))}>Stage選択</button>
      <button disabled={available || commitDraft || !selection.some(path => staged.some(item => item.path === path && !item.conflicted && !item.submodule))} onClick={() => void write("unstage", selection.filter(path => staged.some(item => item.path === path && !item.conflicted && !item.submodule)))}>Unstage選択</button>
      <button className="destructive" disabled={available || commitDraft || !selection.some(path => unstaged.some(item => item.path === path && !item.untracked && !item.conflicted && !item.submodule))} onClick={() => void write("discard", selection.filter(path => unstaged.some(item => item.path === path && !item.untracked && !item.conflicted && !item.submodule)))}>未ステージ変更を破棄</button>
      <button className="primary" disabled={available || commitDraft || staged.length === 0 || !!info?.detached || staged.some(item => item.submodule) || changes.some(item => item.conflicted)} onClick={commit}>Commit ({staged.length})</button>
      <button disabled={available || commitDraft} onClick={() => { const name = window.prompt("新しいブランチ名"); if (name?.trim()) void write("branch", [], name.trim()); }}>ブランチ作成</button>
      <select aria-label="ブランチ切替" disabled={available || commitDraft} value="" onChange={event => { if (event.target.value) void write("switch", [], event.target.value); }}><option value="">ブランチ切替…</option>{branches.filter(branch => !branch.current).map(branch => <option key={branch.name} value={branch.name}>{branch.name}</option>)}</select>
      <button disabled={available || commitDraft} onClick={() => void write("fetch")}>Fetch</button><button disabled={available || commitDraft || !info?.remote || !!info?.detached} onClick={() => void write("pull")}>ff-only Pull</button><button disabled={available || commitDraft || !info?.remote || !info?.head || !!info?.detached} onClick={() => void write("push")}>Push</button>
    </div>
    {commitDraft && <section className="write-confirmation" aria-label="Gitコミット確認"><h2>ステージ済み変更をコミット</h2><div className="write-destination"><strong>{workspace.name} · {info?.branch}</strong><code>{workspace.path}</code></div><p>以下のステージ済み変更すべてをコミットします。未ステージ変更と未追跡ファイルは含みません。</p><ul className="write-targets">{staged.map(item => <li key={item.path}><code>{item.path}</code></li>)}</ul><label>コミットメッセージ<textarea autoFocus value={commitMessage} maxLength={65536} disabled={busy} onChange={event => setCommitMessage(event.target.value)} rows={4} /></label><label className="checkbox-label"><input type="checkbox" checked={commitConfirmed} disabled={busy} onChange={event => setCommitConfirmed(event.target.checked)} />対象一覧とコミットメッセージを確認しました</label><div className="form-actions"><button className="primary" disabled={available || busy || !commitConfirmed || !commitMessage.trim()} onClick={() => void executeCommit()}>{busy ? "コミット中…" : "コミットする"}</button><button disabled={busy} onClick={() => setCommitDraft(false)}>キャンセル</button></div></section>}
    <div className="tabs" role="tablist" aria-label="Git表示"><button role="tab" aria-selected={tab === "changes"} onClick={() => setTab("changes")}>変更 <span>{changes.length}</span></button><button role="tab" aria-selected={tab === "history"} onClick={() => setTab("history")}>履歴</button></div>
    {tab === "changes" ? <section className="git-files"><div><h2>ステージ済み ({staged.length})</h2>{staged.map(change => <GitFile key={`s-${change.path}`} change={change} selected={selection.includes(change.path)} onSelect={checked => setSelection(old => checked ? [...new Set([...old, change.path])] : old.filter(path => path !== change.path))} onOpen={() => void showDiff(change.path, true)} />)}</div><div><h2>未ステージ ({unstaged.length})</h2>{unstaged.map(change => <GitFile key={`u-${change.path}`} change={change} selected={selection.includes(change.path)} onSelect={checked => setSelection(old => checked ? [...new Set([...old, change.path])] : old.filter(path => path !== change.path))} onOpen={() => void showDiff(change.path, false)} />)}</div><div className="git-diff"><h2>{revision ? revision.hash.slice(0, 12) : selected || "差分"}</h2>{revision && <><p>{revision.author} · {date(revision.date)} · {revision.refs}</p><pre>{revision.message}</pre><p>parents: {revision.parents.map(hash => hash.slice(0, 12)).join(", ") || "root"}</p></>}{diff && <pre>{diff}</pre>}</div></section> : <section className="git-history"><div>{history.map((commit, index) => <button key={commit.hash} onClick={() => void showCommit(commit)}><span className="commit-graph" aria-hidden="true"><b>●</b><i>{history.some((item, next) => next > index && item.hash === commit.parents[0]) ? "│" : ""}</i></span><code>{commit.hash.slice(0, 10)}</code><strong>{commit.message}</strong><span>{commit.refs}</span><small>{commit.author} · {date(commit.date)}</small><small>parents: {commit.parents.map(hash => hash.slice(0, 7)).join(", ") || "root"}</small></button>)}{hasMore && <button onClick={() => void pageMore()}>次の100件</button>}</div><div className="git-diff"><h2>{revision?.hash.slice(0, 12) ?? "コミットを選択"}</h2>{revision && <><p>{revision.author} · {date(revision.date)} · {revision.refs}</p><pre>{revision.message}</pre><p>parents: {revision.parents.map(hash => hash.slice(0, 12)).join(", ") || "root"}</p>{diff && <pre>{diff}</pre>}</>}</div></section>}
    <div className="branch-list" aria-label="ブランチ">{branches.map(branch => <span key={branch.name}>{branch.current ? "● " : "○ "}{branch.name}{branch.upstream && ` · ${branch.upstream} ↑${branch.ahead} ↓${branch.behind}`}</span>)}</div>
  </main></div>;
}

function GitFile({ change, selected, onSelect, onOpen }: { change: GitChange; selected: boolean; onSelect: (checked: boolean) => void; onOpen: () => void }) {
  const status = change.submodule ? "submodule · 対象外" : change.conflicted ? "競合" : change.untracked ? "未追跡" : `${change.index === "." ? "" : `Stage:${change.index}`} ${change.worktree === "." ? "" : `Work:${change.worktree}`}`;
  return <div className="selectable-change"><input type="checkbox" aria-label={`${change.path} を選択`} checked={selected} onChange={event => onSelect(event.target.checked)} /><button className="change-row" onClick={onOpen}><span className="status-label">{status}</span><span className="file-path">{change.path}</span></button></div>;
}
