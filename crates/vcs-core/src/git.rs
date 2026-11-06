use crate::{runner, Error, Result, Target, Workspace};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub index: String,
    pub worktree: String,
    pub untracked: bool,
    pub conflicted: bool,
    pub submodule: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub date: String,
    pub message: String,
    pub refs: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub upstream: String,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub version: String,
    pub root: String,
    pub branch: String,
    pub head: String,
    pub detached: bool,
    pub remote: String,
    pub operation: String,
    pub warnings: Vec<String>,
}

pub struct Client {
    pub git: String,
    pub ssh: String,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            git: "git".into(),
            ssh: "ssh".into(),
        }
    }
}

impl Client {
    async fn run(
        &self,
        workspace: &Workspace,
        args: &[&str],
        paths: &[&str],
        limit: usize,
    ) -> Result<String> {
        self.run_input(workspace, args, paths, limit, None).await
    }

    async fn run_input(
        &self,
        workspace: &Workspace,
        args: &[&str],
        paths: &[&str],
        limit: usize,
        input: Option<Vec<u8>>,
    ) -> Result<String> {
        if workspace.vcs != crate::VcsKind::Git {
            return Err(Error::new(
                "wrongVcs",
                "この作業コピーはGitとして登録されていません",
            ));
        }
        let args = args.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        String::from_utf8(
            runner::execute_bytes(
                runner::command_with_tool(workspace, &self.git, &self.ssh, &args, paths)?,
                60,
                limit,
                input,
            )
            .await?,
        )
        .map_err(|_| Error::new("encoding", "Git出力をUTF-8として解釈できません"))
    }

    pub async fn inspect(&self, workspace: &Workspace) -> Result<Info> {
        let version = self.run(workspace, &["--version"], &[], 4096).await?;
        let output = self
            .run(
                workspace,
                &[
                    "status",
                    "--porcelain=v2",
                    "-z",
                    "--branch",
                    "--untracked-files=all",
                ],
                &[],
                1024 * 1024,
            )
            .await?;
        let metadata = output.split('\0').collect::<Vec<_>>();
        let branch = metadata
            .iter()
            .find_map(|field| field.strip_prefix("# branch.head "))
            .unwrap_or("");
        let remote = metadata
            .iter()
            .find_map(|field| field.strip_prefix("# branch.upstream "))
            .unwrap_or("");
        let head = self
            .run(workspace, &["rev-parse", "--verify", "HEAD"], &[], 4096)
            .await
            .unwrap_or_default()
            .trim()
            .to_owned();
        let git_dir = self
            .run(workspace, &["rev-parse", "--git-dir"], &[], 4096)
            .await?;
        let root = self
            .run(workspace, &["rev-parse", "--show-toplevel"], &[], 4096)
            .await?
            .trim()
            .to_owned();
        let git_dir = if std::path::Path::new(git_dir.trim()).is_absolute() {
            std::path::PathBuf::from(git_dir.trim())
        } else {
            std::path::Path::new(&workspace.path).join(git_dir.trim())
        };
        let operation = [
            "MERGE_HEAD",
            "rebase-merge",
            "rebase-apply",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
        ]
        .into_iter()
        .find(|name| git_dir.join(name).exists())
        .unwrap_or("")
        .to_string();
        let head_ref = branch == "(detached)";
        let at_root = match workspace.target {
            Target::Local => match (
                std::fs::canonicalize(&workspace.path),
                std::fs::canonicalize(&root),
            ) {
                (Ok(configured), Ok(actual)) => configured == actual,
                _ => false,
            },
            Target::Ssh { .. } => {
                workspace.path.trim_end_matches('/') == root.trim_end_matches('/')
            }
        };
        Ok(Info {
            version: version.trim().replace("git version ", ""),
            root: root.clone(),
            branch: branch.into(),
            head,
            detached: head_ref,
            remote: remote.into(),
            operation: operation.trim().into(),
            warnings: if at_root {
                Vec::new()
            } else {
                vec![format!(
                    "Git作業ツリーのルートは {root} です。ルートを登録し直すまで書込みを停止します"
                )]
            },
        })
    }

    pub async fn status(&self, workspace: &Workspace) -> Result<Vec<Change>> {
        parse_status(
            &self
                .run(
                    workspace,
                    &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
                    &[],
                    16 * 1024 * 1024,
                )
                .await?,
        )
    }

    pub async fn diff(&self, workspace: &Workspace, path: &str, staged: bool) -> Result<String> {
        runner::validate_relative(path, cfg!(windows) && workspace.target == Target::Local)?;
        let changes = self.status(workspace).await?;
        if !changes.iter().any(|c| c.path == path) {
            return Err(Error::new(
                "staleSelection",
                "対象の状態が変わりました。一覧を再取得してください",
            ));
        }
        if changes
            .iter()
            .find(|c| c.path == path)
            .is_some_and(|c| c.untracked)
        {
            return Err(Error::new(
                "diffUnavailable",
                "未追跡ファイルにはGit差分がありません",
            ));
        }
        if changes
            .iter()
            .find(|c| c.path == path)
            .is_some_and(|c| c.submodule)
        {
            return Err(Error::new(
                "unsupportedWorkspace",
                "submoduleの差分は表示できません",
            ));
        }
        let literal = format!(":(literal){path}");
        let mut args = vec!["diff", "--no-ext-diff", "--", literal.as_str()];
        if staged {
            args.insert(1, "--cached");
        }
        self.run(workspace, &args, &[path], 2 * 1024 * 1024).await
    }

    pub async fn history(&self, workspace: &Workspace, skip: u32) -> Result<Vec<Commit>> {
        if let Err(error) = self
            .run(workspace, &["rev-parse", "--verify", "HEAD"], &[], 4096)
            .await
        {
            let status = self
                .run(
                    workspace,
                    &["status", "--porcelain=v2", "-z", "--branch"],
                    &[],
                    1024 * 1024,
                )
                .await?;
            if status
                .split('\0')
                .any(|field| field == "# branch.oid (initial)")
            {
                return Ok(Vec::new());
            }
            return Err(error);
        }
        let format = "%H%x00%P%x00%an%x00%aI%x00%s%x00%D%x00";
        let skip = skip.to_string();
        let output = self
            .run(
                workspace,
                &[
                    "log",
                    "--all",
                    "--decorate=short",
                    "--date-order",
                    "--max-count=100",
                    "--skip",
                    &skip,
                    "--format",
                    format,
                ],
                &[],
                8 * 1024 * 1024,
            )
            .await?;
        let fields = output
            .split('\0')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if fields.len() % 6 != 0 {
            return Err(Error::new("invalidOutput", "Git履歴出力を解析できません"));
        }
        Ok(fields
            .chunks_exact(6)
            .map(|f| Commit {
                hash: f[0].into(),
                parents: f[1].split_whitespace().map(str::to_owned).collect(),
                author: f[2].into(),
                date: f[3].into(),
                message: f[4].into(),
                refs: f[5].trim().into(),
            })
            .collect())
    }

    pub async fn commit_diff(&self, workspace: &Workspace, hash: &str) -> Result<String> {
        if !(40..=64).contains(&hash.len()) || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::new("invalidRevision", "コミットIDが不正です"));
        }
        self.run(
            workspace,
            &["show", "--format=", "--no-ext-diff", "--no-renames", hash],
            &[],
            2 * 1024 * 1024,
        )
        .await
    }

    pub async fn branches(&self, workspace: &Workspace) -> Result<Vec<Branch>> {
        let format = "%(refname:short)%00%(HEAD)%00%(upstream:short)%00%(ahead-behind:upstream)";
        let output = self
            .run(
                workspace,
                &["for-each-ref", &format!("--format={format}"), "refs/heads"],
                &[],
                1024 * 1024,
            )
            .await?;
        Ok(output
            .lines()
            .filter_map(|line| {
                let f = line.split('\0').collect::<Vec<_>>();
                let (ahead, behind) = f
                    .get(3)
                    .unwrap_or(&"")
                    .split_once(' ')
                    .unwrap_or(("0", "0"));
                Some(Branch {
                    name: (*f.first()?).into(),
                    current: f.get(1) == Some(&"*"),
                    upstream: (*f.get(2).unwrap_or(&"")).into(),
                    ahead: ahead.parse().unwrap_or(0),
                    behind: behind.parse().unwrap_or(0),
                })
            })
            .collect())
    }

    pub async fn write(
        &self,
        workspace: &Workspace,
        action: &str,
        paths: &[String],
        message: &str,
    ) -> Result<String> {
        self.preflight(workspace, action, paths, message).await?;
        let target = if matches!(action, "pull" | "push") && message.is_empty() {
            self.inspect(workspace).await?.remote
        } else {
            message.to_owned()
        };
        match action {
            "stage" | "unstage" => {
                let mut args = vec![
                    if action == "stage" { "add" } else { "reset" }.to_owned(),
                    "--".to_owned(),
                ];
                args.extend(paths.iter().map(|path| format!(":(literal){path}")));
                let args = args.iter().map(String::as_str).collect::<Vec<_>>();
                self.run(
                    workspace,
                    &args,
                    &paths.iter().map(String::as_str).collect::<Vec<_>>(),
                    1024 * 1024,
                )
                .await
            }
            "discard" => {
                let mut args = vec![
                    "restore".to_owned(),
                    "--worktree".to_owned(),
                    "--".to_owned(),
                ];
                args.extend(paths.iter().map(|path| format!(":(literal){path}")));
                let args = args.iter().map(String::as_str).collect::<Vec<_>>();
                self.run(
                    workspace,
                    &args,
                    &paths.iter().map(String::as_str).collect::<Vec<_>>(),
                    1024 * 1024,
                )
                .await
            }
            "commit" => {
                self.run_input(
                    workspace,
                    &["-c", "core.quotepath=false", "commit", "--file", "-"],
                    &[],
                    1024 * 1024,
                    Some(message.as_bytes().to_vec()),
                )
                .await
            }
            "branch" => {
                self.run(workspace, &["switch", "-c", message], &[], 1024 * 1024)
                    .await
            }
            "switch" => {
                self.run(workspace, &["switch", "--", message], &[], 1024 * 1024)
                    .await
            }
            "fetch" => self.run(workspace, &["fetch"], &[], 1024 * 1024).await,
            "pull" | "push" => {
                let mut fields = target.split_whitespace();
                let remote = fields.next().unwrap_or("");
                let branch = fields.next().unwrap_or("");
                if action == "pull" {
                    self.run(
                        workspace,
                        &["pull", "--ff-only", "--", remote, branch],
                        &[],
                        1024 * 1024,
                    )
                    .await
                } else {
                    self.run(workspace, &["push", "--", remote, branch], &[], 1024 * 1024)
                        .await
                }
            }
            _ => Err(Error::new("invalidAction", "未対応のGit操作です")),
        }
    }

    pub async fn preflight(
        &self,
        workspace: &Workspace,
        action: &str,
        paths: &[String],
        message: &str,
    ) -> Result<()> {
        let info = self.inspect(workspace).await?;
        if !info.warnings.is_empty() {
            return Err(Error::new("unsupportedWorkspace", info.warnings.join("\n")));
        }
        if !info.operation.is_empty() {
            return Err(Error::new(
                "operationInProgress",
                format!(
                    "Gitの{}処理を外部で完了または中止してください",
                    info.operation
                ),
            ));
        }
        if matches!(action, "commit" | "pull" | "push") && info.detached {
            return Err(Error::new(
                "detachedHead",
                "detached HEADではこの操作を実行できません",
            ));
        }
        if action == "push" && info.head.is_empty() {
            return Err(Error::new(
                "unbornHead",
                "最初のcommitを作成してからPushしてください",
            ));
        }
        for path in paths {
            runner::validate_relative(path, cfg!(windows) && workspace.target == Target::Local)?;
        }
        if paths.len() > 200 || paths.iter().map(String::len).sum::<usize>() > 16000 {
            return Err(Error::new(
                "selectionLimit",
                "一度の操作は200対象・パス合計16000バイトまでです",
            ));
        }
        if message.len() > 65536 || message.contains('\0') {
            return Err(Error::new(
                "invalidInput",
                "Git操作の入力が上限を超えています",
            ));
        }
        if matches!(action, "stage" | "unstage" | "discard") && paths.is_empty() {
            return Err(Error::new(
                "invalidSelection",
                "対象ファイルを選択してください",
            ));
        }
        if matches!(action, "stage" | "unstage" | "discard") {
            let status = self.status(workspace).await?;
            for path in paths {
                let change = status
                    .iter()
                    .find(|change| change.path == *path)
                    .ok_or_else(|| {
                        Error::new(
                            "staleSelection",
                            "対象の状態が変わりました。一覧を再取得してください",
                        )
                    })?;
                if change.submodule {
                    return Err(Error::new(
                        "unsupportedWorkspace",
                        "submoduleの変更はこのアプリから操作できません",
                    ));
                }
                let allowed = match action {
                    "stage" => (change.worktree != "." || change.untracked) && !change.conflicted,
                    "unstage" => change.index != "." && !change.conflicted,
                    "discard" => change.worktree != "." && !change.untracked && !change.conflicted,
                    _ => false,
                };
                if !allowed {
                    return Err(Error::new(
                        "invalidSelection",
                        format!("この操作に対応しない状態です: {path}"),
                    ));
                }
            }
        }
        if action == "commit"
            && (message.trim().is_empty() || message.len() > 65536 || message.contains('\0'))
        {
            return Err(Error::new(
                "invalidMessage",
                "コミットメッセージを入力してください",
            ));
        }
        if action == "commit" {
            let status = self.status(workspace).await?;
            if status.iter().any(|change| change.conflicted) {
                return Err(Error::new("conflict", "競合を外部で解決してください"));
            }
            if status
                .iter()
                .any(|change| change.submodule && change.index != ".")
            {
                return Err(Error::new(
                    "unsupportedWorkspace",
                    "submoduleがステージされています。submoduleを外部で処理してください",
                ));
            }
        }
        if action == "commit"
            && self
                .run(
                    workspace,
                    &["diff", "--cached", "--name-only", "-z"],
                    &[],
                    1024 * 1024,
                )
                .await?
                .is_empty()
        {
            return Err(Error::new(
                "nothingStaged",
                "ステージ済みの変更がありません",
            ));
        }
        if action == "branch" {
            self.run(
                workspace,
                &["check-ref-format", "--branch", message],
                &[],
                4096,
            )
            .await?;
        }
        if action == "pull" || action == "push" {
            let target = if message.is_empty() {
                &info.remote
            } else {
                message
            };
            let fields = target.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 2 || fields[0].starts_with('-') || fields[1].starts_with('-') {
                return Err(Error::new(
                    "invalidRemote",
                    "remote名とbranch名を指定してください",
                ));
            }
            self.run(
                workspace,
                &["check-ref-format", "--branch", fields[1]],
                &[],
                4096,
            )
            .await?;
        }
        Ok(())
    }
}

pub fn parse_status(output: &str) -> Result<Vec<Change>> {
    let mut changes = Vec::new();
    let mut fields = output.split('\0');
    while let Some(record) = fields.next() {
        if record.is_empty() || record.starts_with('#') {
            continue;
        }
        let kind = record.as_bytes()[0];
        let (index, worktree, path, conflicted, untracked, submodule) = match kind {
            b'1' => {
                let f = record.splitn(9, ' ').collect::<Vec<_>>();
                if f.len() != 9 {
                    return Err(invalid_status());
                }
                (
                    f[1].chars().next().unwrap_or('.'),
                    f[1].chars().nth(1).unwrap_or('.'),
                    f[8],
                    false,
                    false,
                    f[2] != "N...",
                )
            }
            b'2' => {
                let f = record.splitn(10, ' ').collect::<Vec<_>>();
                if f.len() != 10 {
                    return Err(invalid_status());
                }
                let path = f[9];
                let _original_path = fields.next().ok_or_else(invalid_status)?;
                (
                    f[1].chars().next().unwrap_or('.'),
                    f[1].chars().nth(1).unwrap_or('.'),
                    path,
                    false,
                    false,
                    f[2] != "N...",
                )
            }
            b'u' => {
                let f = record.splitn(12, ' ').collect::<Vec<_>>();
                if f.len() != 12 {
                    return Err(invalid_status());
                }
                (
                    f[1].chars().next().unwrap_or('U'),
                    f[1].chars().nth(1).unwrap_or('U'),
                    f[11],
                    true,
                    false,
                    f[2] != "N...",
                )
            }
            b'?' => (
                '.',
                '?',
                record.get(2..).ok_or_else(invalid_status)?,
                false,
                true,
                false,
            ),
            b'!' => continue,
            _ => return Err(invalid_status()),
        };
        changes.push(Change {
            path: path.into(),
            index: index.to_string(),
            worktree: worktree.to_string(),
            untracked,
            conflicted,
            submodule,
        });
    }
    Ok(changes)
}

fn invalid_status() -> Error {
    Error::new("invalidOutput", "Git status出力を解析できません")
}
