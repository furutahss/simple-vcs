use crate::{
    runner,
    svn::{parse, parse_status, strings, Change, Client},
    Error, Result, Target, Workspace,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteAction {
    Add,
    Delete,
    Commit,
    Update,
    Revert,
    Cleanup,
    Resolve,
    Stage,
    Unstage,
    Branch,
    Switch,
    Fetch,
    Pull,
    Push,
    Discard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteRequest {
    pub action: WriteAction,
    pub paths: Vec<String>,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug)]
pub struct PreparedWrite {
    pub workspace: Workspace,
    pub request: WriteRequest,
    pub paths: Vec<String>,
    pub warnings: Vec<String>,
    snapshot: Snapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResult {
    pub output: String,
    pub revision: Option<u64>,
}

impl Client {
    pub async fn prepare_write(
        &self,
        workspace: &Workspace,
        mut request: WriteRequest,
    ) -> Result<PreparedWrite> {
        runner::validate(workspace)?;
        if workspace.vcs != crate::VcsKind::Svn {
            return Err(Error::new(
                "wrongVcs",
                "SVN操作はSVN作業コピーでのみ実行できます",
            ));
        }
        if request.message.len() > 65536
            || request.message.contains('\0')
            || (request.action == WriteAction::Commit && request.message.trim().is_empty())
        {
            return Err(Error::new(
                "invalidMessage",
                "コミットメッセージを1～65536バイトで入力してください",
            ));
        }
        if matches!(request.action, WriteAction::Update | WriteAction::Cleanup) {
            if !request.paths.is_empty() {
                return Err(Error::new(
                    "invalidSelection",
                    "更新・cleanupは作業コピー全体を対象にします",
                ));
            }
        } else if request.paths.is_empty() {
            return Err(Error::new("invalidSelection", "操作対象を選択してください"));
        }
        if request.paths.len() > 200 || request.paths.iter().map(String::len).sum::<usize>() > 16000
        {
            return Err(Error::new(
                "selectionLimit",
                "一度の操作は200対象・パス合計16000バイトまでです",
            ));
        }
        for path in &request.paths {
            runner::validate_relative(path, cfg!(windows) && workspace.target == Target::Local)?;
        }
        request.paths.sort();
        request.paths.dedup();
        let info = self.inspect(workspace).await?;
        if !info.warnings.is_empty() {
            return Err(Error::new("unsupportedWorkspace", info.warnings.join("\n")));
        }
        let info_xml = self
            .run(
                workspace,
                strings(&[
                    "info",
                    "--xml",
                    "--depth",
                    "infinity",
                    "--non-interactive",
                    "--",
                    ".@",
                ]),
                None,
                SNAPSHOT_LIMIT,
            )
            .await?;
        let mut nodes: Nodes = parse(&info_xml, b"info")?;
        nodes.entries.sort_by(|a, b| a.path.cmp(&b.path));
        if nodes.entries.iter().any(|node| {
            node.kind == "dir" && node.wc.as_ref().is_none_or(|wc| wc.depth != "infinity")
        }) {
            return Err(unsupported("sparse作業コピーの書込みには対応していません"));
        }
        let properties = self
            .run(
                workspace,
                strings(&[
                    "propget",
                    "svn:externals",
                    "--xml",
                    "--recursive",
                    "--non-interactive",
                    "--",
                    ".@",
                ]),
                None,
                SNAPSHOT_LIMIT,
            )
            .await?;
        let external: Properties = parse(&properties, b"properties")?;
        if !external.targets.is_empty() {
            return Err(unsupported(
                "externals定義がある作業コピーの書込みには対応していません",
            ));
        }
        let status_xml = self
            .run(
                workspace,
                strings(&[
                    "status",
                    "--xml",
                    "--verbose",
                    "--no-ignore",
                    "--ignore-externals",
                    "--non-interactive",
                    "--",
                    ".@",
                ]),
                None,
                SNAPSHOT_LIMIT,
            )
            .await?;
        let mut changes = parse_status(
            &status_xml,
            cfg!(windows) && workspace.target == Target::Local,
        )?;
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        for change in &changes {
            if change.switched
                || change.copied
                || change.status == "external"
                || change.tree_conflicted
                || change.properties == "conflicted"
                || ["obstructed", "incomplete"].contains(&change.status.as_str())
            {
                return Err(unsupported("切替・コピー／移動・外部作業コピー・ツリー／プロパティ競合等を外部CLIで処理してください"));
            }
            if change.locked && request.action != WriteAction::Cleanup {
                return Err(unsupported(
                    "作業コピーがロックされています。cleanupを確認してください",
                ));
            }
            if change.status == "conflicted" && request.action != WriteAction::Resolve {
                return Err(unsupported(
                    "テキスト競合を外部で解決してから、解決済みにしてください",
                ));
            }
        }
        let whole = matches!(request.action, WriteAction::Update | WriteAction::Cleanup);
        let mut paths = if whole {
            vec![".".into()]
        } else {
            request.paths.clone()
        };
        let selected: HashSet<_> = paths.iter().map(String::as_str).collect();
        for path in &paths {
            let change = changes.iter().find(|c| c.path == *path).ok_or_else(|| {
                Error::new(
                    "staleSelection",
                    "対象の状態が変わりました。一覧を再取得してください",
                )
            })?;
            let permitted = match request.action {
                WriteAction::Add => change.status == "unversioned",
                WriteAction::Delete => {
                    path != "."
                        && ["normal", "missing"].contains(&change.status.as_str())
                        && ["none", "normal"].contains(&change.properties.as_str())
                }
                WriteAction::Commit | WriteAction::Revert => {
                    ["modified", "added", "deleted", "replaced"].contains(&change.status.as_str())
                        || (change.status == "normal" && change.properties == "modified")
                }
                WriteAction::Resolve => {
                    change.status == "conflicted"
                        && !change.tree_conflicted
                        && change.properties != "conflicted"
                }
                WriteAction::Update | WriteAction::Cleanup => true,
                _ => unreachable!("Git操作は上で拒否済み"),
            };
            if !permitted {
                return Err(Error::new(
                    "invalidSelection",
                    format!("この操作に対応しない状態です: {path} ({})", change.status),
                ));
            }
            if request.action == WriteAction::Revert {
                for ancestor in ancestors(path) {
                    if changes
                        .iter()
                        .any(|c| c.path == ancestor && c.status == "deleted")
                        && !selected.contains(ancestor)
                    {
                        return Err(Error::new(
                            "parentRequired",
                            format!("削除予定の親ディレクトリも選択してください: {ancestor}"),
                        ));
                    }
                }
                if change.status == "added"
                    && changes.iter().any(|c| {
                        below(&c.path, path)
                            && c.status == "added"
                            && !selected.contains(c.path.as_str())
                    })
                {
                    return Err(Error::new(
                        "childrenRequired",
                        "追加予定ディレクトリの取消には、追加予定の子も選択してください",
                    ));
                }
            }
            if request.action == WriteAction::Commit {
                for ancestor in ancestors(path) {
                    if changes
                        .iter()
                        .any(|c| c.path == ancestor && c.status == "added")
                        && !selected.contains(ancestor)
                    {
                        return Err(Error::new(
                            "parentRequired",
                            format!("追加予定の親ディレクトリも選択してください: {ancestor}"),
                        ));
                    }
                }
                if change.status == "deleted"
                    && changes.iter().any(|c| {
                        below(&c.path, path)
                            && c.status == "deleted"
                            && !selected.contains(c.path.as_str())
                    })
                {
                    return Err(Error::new(
                        "childrenRequired",
                        "ディレクトリ削除のコミットには、削除予定の子もすべて選択してください",
                    ));
                }
            }
            if request.action == WriteAction::Delete
                && changes.iter().any(|c| {
                    below(&c.path, path)
                        && (c.status != "normal"
                            || !["none", "normal"].contains(&c.properties.as_str()))
                })
            {
                return Err(Error::new(
                    "localChanges",
                    "配下に変更・未追跡・無視ファイルがあります。削除できません",
                ));
            }
        }
        let mut guard_paths: Vec<_> = if whole {
            changes
                .iter()
                .filter(|c| {
                    !["unversioned", "ignored", "external"].contains(&c.status.as_str())
                        && (c.status != "normal" || c.properties == "modified")
                })
                .map(|c| c.path.as_str())
                .collect()
        } else {
            paths.iter().map(String::as_str).collect()
        };
        if whole && !guard_paths.contains(&".") {
            guard_paths.push(".");
        }
        if guard_paths.iter().map(|p| p.len()).sum::<usize>() > 16000 {
            return Err(Error::new(
                "selectionLimit",
                "安全確認するパスが多すぎます。対象を分けてください",
            ));
        }
        let guard = runner::command_for_paths(
            workspace,
            &self.svn,
            &self.ssh,
            &strings(&["--version", "--quiet"]),
            &guard_paths,
        )?;
        runner::execute(guard, 60, 4096).await?;
        let mut args = strings(&[
            "diff",
            "--internal-diff",
            "--force",
            "--non-interactive",
            "--depth",
            if whole { "infinity" } else { "empty" },
        ]);
        args.push("--".into());
        args.extend(
            paths
                .iter()
                .filter(|path| {
                    changes.iter().any(|c| {
                        c.path == **path
                            && !["unversioned", "ignored", "external"].contains(&c.status.as_str())
                    })
                })
                .map(|p| format!("./{p}")),
        );
        let diff = if request.action == WriteAction::Add || request.action == WriteAction::Cleanup {
            Vec::new()
        } else {
            runner::execute_bytes(
                runner::command_for_paths(workspace, &self.svn, &self.ssh, &args, &guard_paths)?,
                60,
                SNAPSHOT_LIMIT,
                None,
            )
            .await?
        };
        if request.action == WriteAction::Delete {
            for node in &nodes.entries {
                let path = normalize_path(&node.path, workspace);
                if request.paths.iter().any(|parent| below(&path, parent)) {
                    paths.push(path);
                }
            }
            paths.sort();
            paths.dedup();
            if paths.len() > 200 || paths.iter().map(String::len).sum::<usize>() > 16000 {
                return Err(Error::new(
                    "selectionLimit",
                    "配下を含めた削除対象が上限を超えています。対象を分けてください",
                ));
            }
        }
        let warnings = match request.action {
            WriteAction::Commit => vec!["選択対象をSVNリポジトリへ送信します。未追跡ファイルを自動追加しません。".into()],
            WriteAction::Revert => vec!["選択対象のローカル変更は失われます。未追跡ファイルは削除しません。".into()],
            WriteAction::Delete => vec!["選択したファイル・ディレクトリは実行先のディスクから削除されます。ディレクトリの配下も対象です。".into()],
            WriteAction::Update => vec!["作業コピー全体を更新します。ローカル変更とマージされ、競合が発生する場合があります。".into()],
            WriteAction::Resolve => vec!["外部エディタでテキスト競合を解決済みであることを確認してください。現在の内容を解決結果として採用します。".into()],
            WriteAction::Add => vec!["選択対象だけを追加予定にします。ディレクトリの配下は自動追加しません。".into()],
            WriteAction::Cleanup => vec!["通常のcleanupのみ実行します。未追跡・無視ファイルの削除や強制解除は行いません。".into()],
            _ => unreachable!("Git操作は上で拒否済み"),
        };
        Ok(PreparedWrite {
            workspace: workspace.clone(),
            request,
            paths,
            warnings,
            snapshot: Snapshot {
                nodes: nodes.entries,
                changes,
                diff,
            },
        })
    }

    pub async fn verify_write(&self, prepared: &PreparedWrite) -> Result<()> {
        let fresh = self.prepare_write(&prepared.workspace, prepared.request.clone()).await.map_err(|_| Error::new("stalePreview", "確認後に状態が変わったか、再確認できませんでした。プレビューを作り直してください"))?;
        if fresh.snapshot != prepared.snapshot || fresh.paths != prepared.paths {
            return Err(Error::new(
                "stalePreview",
                "確認後に状態が変わりました。プレビューを作り直してください",
            ));
        }
        Ok(())
    }

    pub async fn execute_write(&self, prepared: &PreparedWrite) -> Result<WriteResult> {
        let mut args = match prepared.request.action {
            WriteAction::Add => strings(&["add", "--depth", "empty"]),
            WriteAction::Delete => strings(&["delete"]),
            WriteAction::Commit => strings(&[
                "commit",
                "--depth",
                "empty",
                "--encoding",
                "UTF-8",
                "--no-unlock",
            ]),
            WriteAction::Update => {
                strings(&["update", "--accept", "postpone", "--ignore-externals"])
            }
            WriteAction::Revert => strings(&["revert", "--depth", "empty"]),
            WriteAction::Cleanup => strings(&["cleanup"]),
            WriteAction::Resolve => {
                strings(&["resolve", "--accept", "working", "--depth", "empty"])
            }
            _ => unreachable!("Git操作は上で拒否済み"),
        };
        let mut message_file = None;
        let input = if prepared.request.action == WriteAction::Commit {
            let path = match prepared.workspace.target {
                Target::Local => {
                    let mut file = tempfile::NamedTempFile::new()
                        .map_err(|_| Error::new("io", "メッセージ一時ファイルを作成できません"))?;
                    file.write_all(prepared.request.message.as_bytes())
                        .map_err(|_| Error::new("io", "メッセージを一時保存できません"))?;
                    let path = file
                        .path()
                        .to_str()
                        .ok_or_else(|| {
                            Error::new("encoding", "一時ファイルのパスをUTF-8で表せません")
                        })?
                        .to_string();
                    message_file = Some(file);
                    path
                }
                Target::Ssh { .. } => "/dev/stdin".into(),
            };
            args.extend(["--file".into(), path]);
            if matches!(prepared.workspace.target, Target::Ssh { .. }) {
                Some(prepared.request.message.as_bytes().to_vec())
            } else {
                None
            }
        } else {
            None
        };
        args.extend(strings(&["--non-interactive", "--"]));
        let mut targets = prepared.request.paths.clone();
        if targets.is_empty() {
            targets.push(".".into());
        }
        if prepared.request.action == WriteAction::Revert {
            targets.sort_by_key(|path| {
                let depth = path.split('/').count() as i32;
                if prepared
                    .snapshot
                    .changes
                    .iter()
                    .any(|c| c.path == *path && c.status == "added")
                {
                    -depth
                } else {
                    depth
                }
            });
        }
        args.extend(targets.iter().map(|p| format!("./{p}@")));
        let refs: Vec<_> = targets.iter().map(String::as_str).collect();
        let command =
            runner::command_for_paths(&prepared.workspace, &self.svn, &self.ssh, &args, &refs)?;
        let bytes = runner::execute_bytes(command, 120, SNAPSHOT_LIMIT, input)
            .await
            .map_err(|e| {
                Error::new(
                    "outcomeUnknown",
                    format!(
                        "成否を確認できません。再実行せず、状態と履歴を確認してください。{}",
                        e.message
                    ),
                )
            })?;
        drop(message_file);
        let output = String::from_utf8(bytes).map_err(|_| {
            Error::new(
                "outcomeUnknown",
                "処理は終了しましたが結果を解釈できません。状態と履歴を確認してください",
            )
        })?;
        let revision = output.lines().find_map(|line| {
            line.strip_prefix("Committed revision ")
                .and_then(|v| v.strip_suffix('.'))
                .and_then(|v| v.parse().ok())
        });
        let after = self.status(&prepared.workspace).await.map_err(|_| {
            Error::new(
                "outcomeUnknown",
                "CLIは終了しましたが、変更後の状態を確認できません。成否を確認してください",
            )
        })?;
        for path in &prepared.paths {
            let current = after.iter().find(|c| c.path == *path);
            let expected = match prepared.request.action {
                WriteAction::Add => current.is_some_and(|c| c.status == "added"),
                WriteAction::Delete => {
                    current.is_some_and(|c| c.status == "deleted")
                        || after
                            .iter()
                            .any(|c| c.status == "deleted" && below(path, &c.path))
                }
                WriteAction::Commit | WriteAction::Revert => current.is_none_or(|c| {
                    ["normal", "unversioned"].contains(&c.status.as_str())
                        && ["none", "normal"].contains(&c.properties.as_str())
                }),
                WriteAction::Resolve => current.is_none_or(|c| {
                    c.status != "conflicted" && !c.tree_conflicted && c.properties != "conflicted"
                }),
                WriteAction::Cleanup => !after.iter().any(|c| c.locked),
                WriteAction::Update => true,
                _ => unreachable!("Git操作は上で拒否済み"),
            };
            if !expected {
                return Err(Error::new("outcomeUnknown", format!("CLIは終了しましたが、対象の処理完了を確認できません: {path}。状態・履歴を確認してください")));
            }
        }
        Ok(WriteResult { output, revision })
    }
}

// shortcut: 差分の照合は最大16 MiB。大きな変更が必要ならストリームハッシュへ変更する。
const SNAPSHOT_LIMIT: usize = 16 * 1024 * 1024;

fn unsupported(message: &str) -> Error {
    Error::new("unsupportedWorkspace", message)
}
fn below(path: &str, parent: &str) -> bool {
    path != parent && (parent == "." || path.starts_with(&format!("{parent}/")))
}
fn ancestors(path: &str) -> Vec<&str> {
    path.match_indices('/')
        .map(|(index, _)| &path[..index])
        .chain((path != ".").then_some("."))
        .collect()
}
fn normalize_path(path: &str, workspace: &Workspace) -> String {
    let path = if cfg!(windows) && workspace.target == Target::Local {
        path.replace('\\', "/")
    } else {
        path.to_string()
    };
    path.strip_prefix("./").unwrap_or(&path).to_string()
}

#[derive(Deserialize)]
struct Nodes {
    #[serde(rename = "entry")]
    entries: Vec<Node>,
}
#[derive(Debug, PartialEq, Eq, Deserialize)]
struct Node {
    #[serde(rename = "@path")]
    path: String,
    #[serde(rename = "@kind")]
    kind: String,
    #[serde(rename = "@revision")]
    revision: Option<String>,
    url: String,
    #[serde(rename = "wc-info")]
    wc: Option<NodeWc>,
}
#[derive(Debug, PartialEq, Eq, Deserialize)]
struct NodeWc {
    #[serde(default)]
    depth: String,
    #[serde(rename = "wcroot-abspath")]
    root: String,
    schedule: String,
    #[serde(default)]
    checksum: String,
}
#[derive(Deserialize)]
struct Properties {
    #[serde(default, rename = "target")]
    targets: Vec<PropertyTarget>,
}
#[derive(Deserialize)]
struct PropertyTarget {}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    nodes: Vec<Node>,
    changes: Vec<Change>,
    diff: Vec<u8>,
}
