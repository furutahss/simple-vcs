use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vcs_core::{
    svn::Client,
    writes::{PreparedWrite, WriteAction, WriteRequest, WriteResult},
    Error, Result, Target, Workspace,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingOperation {
    pub id: String,
    pub workspace: Workspace,
    pub action: WriteAction,
    pub paths: Vec<String>,
    pub started_at: u64,
    key: String,
}

pub struct Journal {
    pub path: PathBuf,
}
impl Journal {
    pub fn load(&self) -> Result<Vec<PendingOperation>> {
        let Some(bytes) = crate::storage::read(&self.path)? else {
            return Ok(Vec::new());
        };
        let data: JournalData = serde_json::from_slice(&bytes).map_err(|_| Error::new("journal", "未完了操作の記録が破損しています。書込みを停止します。記録を確認して復旧してください"))?;
        if data.version != 1 || data.pending.len() > 200 {
            return Err(Error::new(
                "journal",
                "未対応の操作記録です。自動上書きは行いません",
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for operation in &data.pending {
            vcs_core::runner::validate(&operation.workspace)?;
            if operation.id.is_empty()
                || operation.id.len() > 200
                || !ids.insert(&operation.id)
                || operation.key.is_empty()
            {
                return Err(Error::new(
                    "journal",
                    "操作記録の識別情報が不正です。書込みを停止します",
                ));
            }
        }
        Ok(data.pending)
    }
    fn save(&self, pending: Vec<PendingOperation>) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&JournalData {
            version: 1,
            pending,
        })
        .map_err(|_| Error::new("journal", "操作記録を変換できません"))?;
        crate::storage::write(&self.path, &bytes)
    }
    pub fn begin(&self, operation: PendingOperation) -> Result<()> {
        let mut pending = self.load()?;
        self.ensure_available(&operation.workspace)?;
        if pending.iter().any(|p| p.id == operation.id) {
            return Err(Error::new("journal", "操作IDが重複しています"));
        }
        pending.push(operation);
        self.save(pending)
    }
    pub fn acknowledge(&self, id: &str, confirmed: bool) -> Result<()> {
        if !confirmed {
            return Err(Error::new(
                "confirmationRequired",
                "外部で実行中プロセスの停止と成否を確認してください",
            ));
        }
        let mut pending = self.load()?;
        if !pending.iter().any(|p| p.id == id) {
            return Err(Error::new("operationMissing", "未完了操作が見つかりません"));
        }
        pending.retain(|p| p.id != id);
        self.save(pending)
    }
    pub fn ensure_available(&self, workspace: &Workspace) -> Result<()> {
        let pending = self.load()?;
        if pending.is_empty() {
            return Ok(());
        }
        let execution_key = key(workspace)?;
        if pending
            .iter()
            .any(|p| p.workspace.id == workspace.id || p.key == execution_key)
        {
            return Err(Error::new("pendingOperation", "成否不明の操作があります。状態・履歴と実行先プロセスを確認するまで書込みできません"));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalData {
    version: u32,
    pending: Vec<PendingOperation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WritePreview {
    pub token: String,
    pub workspace: Workspace,
    pub action: WriteAction,
    pub paths: Vec<String>,
    pub warnings: Vec<String>,
}

pub struct Operations {
    pub journal: Journal,
    preview: Option<(String, PreparedWrite, std::time::Instant)>,
}
impl Operations {
    pub fn new(path: PathBuf) -> Self {
        Self {
            journal: Journal { path },
            preview: None,
        }
    }
    pub fn invalidate_preview(&mut self) {
        self.preview = None;
    }
    pub async fn prepare(
        &mut self,
        workspace: &Workspace,
        client: &Client,
        request: WriteRequest,
    ) -> Result<WritePreview> {
        self.invalidate_preview();
        self.journal.ensure_available(workspace)?;
        let prepared = client.prepare_write(workspace, request).await?;
        let token = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::new("clock", "現在時刻を取得できません"))?
                .as_nanos()
        );
        let preview = WritePreview {
            token: token.clone(),
            workspace: workspace.clone(),
            action: prepared.request.action,
            paths: prepared.paths.clone(),
            warnings: prepared.warnings.clone(),
        };
        self.preview = Some((token, prepared, std::time::Instant::now()));
        Ok(preview)
    }
    pub async fn execute(
        &mut self,
        token: &str,
        workspace: &Workspace,
        client: &Client,
    ) -> Result<WriteResult> {
        let (expected, prepared, created) = self.preview.take().ok_or_else(|| {
            Error::new(
                "previewMissing",
                "プレビューがないか、既に実行済みです。確認からやり直してください",
            )
        })?;
        if token != expected
            || prepared.workspace != *workspace
            || created.elapsed().as_secs() >= 300
        {
            return Err(Error::new(
                "stalePreview",
                "確認内容が古いか、登録内容が変わりました。確認からやり直してください",
            ));
        }
        self.journal.ensure_available(workspace)?;
        client.verify_write(&prepared).await?;
        self.journal.begin(PendingOperation {
            id: expected.clone(),
            workspace: workspace.clone(),
            action: prepared.request.action,
            paths: prepared.paths.clone(),
            key: key(workspace)?,
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::new("clock", "現在時刻を取得できません"))?
                .as_secs(),
        })?;
        let result = client.execute_write(&prepared).await.map_err(|error| {
            Error::new(
                "outcomeUnknown",
                format!(
                    "操作の成否を確認してください。自動再実行はしません。{}",
                    error.message
                ),
            )
        })?;
        self.journal.acknowledge(&expected, true).map_err(|_| {
            Error::new(
                "outcomeUnknown",
                "CLIは成功しましたが記録の更新に失敗しました。状態・履歴を確認してください",
            )
        })?;
        Ok(result)
    }

    pub async fn execute_git(
        &mut self,
        workspace: &Workspace,
        client: &vcs_core::git::Client,
        action: WriteAction,
        paths: Vec<String>,
        message: String,
    ) -> Result<String> {
        self.journal.ensure_available(workspace)?;
        let action_name = match action {
            WriteAction::Stage => "stage",
            WriteAction::Unstage => "unstage",
            WriteAction::Branch => "branch",
            WriteAction::Switch => "switch",
            WriteAction::Fetch => "fetch",
            WriteAction::Pull => "pull",
            WriteAction::Push => "push",
            WriteAction::Discard => "discard",
            WriteAction::Commit => "commit",
            _ => return Err(Error::new("invalidAction", "Git操作が不正です")),
        };
        client
            .preflight(workspace, action_name, &paths, &message)
            .await?;
        let id = format!(
            "git-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::new("clock", "現在時刻を取得できません"))?
                .as_nanos()
        );
        self.journal.begin(PendingOperation {
            id: id.clone(),
            workspace: workspace.clone(),
            action,
            paths: paths.clone(),
            key: key(workspace)?,
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::new("clock", "現在時刻を取得できません"))?
                .as_secs(),
        })?;
        match client.write(workspace, action_name, &paths, &message).await {
            Ok(output) => {
                self.journal.acknowledge(&id, true).map_err(|_| {
                    Error::new(
                        "outcomeUnknown",
                        "Git操作は成功しましたが操作記録を更新できません。成否を確認してください",
                    )
                })?;
                Ok(output)
            }
            Err(error) => Err(Error::new(
                "outcomeUnknown",
                format!(
                    "操作の成否を確認してください。自動再実行はしません。{}",
                    error.message
                ),
            )),
        }
    }
}

fn key(workspace: &Workspace) -> Result<String> {
    match &workspace.target {
        Target::Local => {
            let root = std::fs::canonicalize(&workspace.path)
                .map_err(|_| Error::new("invalidPath", "作業コピーの場所を確認できません"))?;
            let root = root
                .to_str()
                .ok_or_else(|| Error::new("encoding", "作業コピーパスをUTF-8で表せません"))?;
            Ok(format!(
                "local\0{}",
                if cfg!(windows) {
                    root.to_lowercase()
                } else {
                    root.to_string()
                }
            ))
        }
        Target::Ssh { host } => Ok(format!(
            "ssh\0{host}\0{}",
            workspace.path.trim_end_matches('/')
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn workspace(path: &str) -> Workspace {
        Workspace {
            id: "one".into(),
            name: "テスト".into(),
            target: Target::Local,
            path: path.into(),
            vcs: vcs_core::VcsKind::Svn,
        }
    }
    fn pending(workspace: Workspace) -> PendingOperation {
        PendingOperation {
            key: key(&workspace).unwrap(),
            workspace,
            id: "operation-1".into(),
            action: WriteAction::Commit,
            paths: vec!["file.txt".into()],
            started_at: 1,
        }
    }

    #[test]
    fn interrupted_operation_survives_restart_and_blocks_alias_registration() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("operations.json");
        let journal = Journal { path: path.clone() };
        let mut workspace = workspace(temp.path().to_str().unwrap());
        journal.begin(pending(workspace.clone())).unwrap();
        let restarted = Journal { path };
        assert_eq!(restarted.load().unwrap().len(), 1);
        workspace.id = "alias".into();
        assert!(restarted.ensure_available(&workspace).is_err());
        assert!(restarted.acknowledge("operation-1", false).is_err());
        assert!(restarted.acknowledge("wrong-id", true).is_err());
        restarted.acknowledge("operation-1", true).unwrap();
        assert!(restarted.ensure_available(&workspace).is_ok());
    }

    #[test]
    fn corrupt_journal_is_not_reset_or_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("operations.json");
        std::fs::write(&path, "{broken").unwrap();
        let journal = Journal { path: path.clone() };
        assert!(journal
            .begin(pending(workspace(temp.path().to_str().unwrap())))
            .is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{broken");
    }

    #[test]
    fn journal_contains_only_operation_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal {
            path: temp.path().join("operations.json"),
        };
        journal
            .begin(pending(workspace(temp.path().to_str().unwrap())))
            .unwrap();
        let bytes = std::fs::read_to_string(journal.path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&bytes).unwrap();
        assert!(value["pending"][0].get("message").is_none());
        assert!(value["pending"][0].get("snapshot").is_none());
        assert!(value["pending"][0].get("diff").is_none());
    }

    fn fixture() -> (tempfile::TempDir, Workspace) {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let work = temp.path().join("work");
        assert!(std::process::Command::new("svnadmin")
            .arg("create")
            .arg(&repo)
            .status()
            .unwrap()
            .success());
        let path = repo.to_str().unwrap().replace('\\', "/");
        let url = format!(
            "file://{}{path}",
            if path.starts_with('/') { "" } else { "/" }
        );
        assert!(std::process::Command::new("svn")
            .args(["checkout", &url, work.to_str().unwrap()])
            .output()
            .unwrap()
            .status
            .success());
        std::fs::write(work.join("new.txt"), "new").unwrap();
        let workspace = workspace(work.to_str().unwrap());
        (temp, workspace)
    }

    #[tokio::test]
    async fn successful_write_clears_journal_and_token_is_single_use() {
        let (temp, workspace) = fixture();
        let mut operations = Operations::new(temp.path().join("operations.json"));
        let client = Client::default();
        let preview = operations
            .prepare(
                &workspace,
                &client,
                WriteRequest {
                    action: WriteAction::Add,
                    paths: vec!["new.txt".into()],
                    message: String::new(),
                },
            )
            .await
            .unwrap();
        operations
            .execute(&preview.token, &workspace, &client)
            .await
            .unwrap();
        assert!(operations.journal.load().unwrap().is_empty());
        assert!(operations
            .execute(&preview.token, &workspace, &client)
            .await
            .is_err());
        assert!(client
            .status(&workspace)
            .await
            .unwrap()
            .iter()
            .any(|c| c.path == "new.txt" && c.status == "added"));
    }

    #[tokio::test]
    async fn stale_preview_does_not_start_or_leave_an_unfinished_operation() {
        let (temp, workspace) = fixture();
        let mut operations = Operations::new(temp.path().join("operations.json"));
        let client = Client::default();
        let preview = operations
            .prepare(
                &workspace,
                &client,
                WriteRequest {
                    action: WriteAction::Add,
                    paths: vec!["new.txt".into()],
                    message: String::new(),
                },
            )
            .await
            .unwrap();
        std::fs::write(
            std::path::Path::new(&workspace.path).join("another.txt"),
            "another",
        )
        .unwrap();
        assert_eq!(
            operations
                .execute(&preview.token, &workspace, &client)
                .await
                .unwrap_err()
                .kind,
            "stalePreview"
        );
        assert!(operations.journal.load().unwrap().is_empty());
        assert!(client
            .status(&workspace)
            .await
            .unwrap()
            .iter()
            .any(|c| c.path == "new.txt" && c.status == "unversioned"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_commit_remains_pending_after_restart_and_cannot_be_retried() {
        use std::os::unix::fs::PermissionsExt;
        let (temp, workspace) = fixture();
        let root = std::path::Path::new(&workspace.path);
        assert!(std::process::Command::new("svn")
            .current_dir(root)
            .args(["add", "new.txt"])
            .output()
            .unwrap()
            .status
            .success());
        let hook = temp.path().join("repo/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\necho denied >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = temp.path().join("operations.json");
        let mut operations = Operations::new(path.clone());
        let client = Client::default();
        let request = WriteRequest {
            action: WriteAction::Commit,
            paths: vec!["new.txt".into()],
            message: "not persisted secret".into(),
        };
        let preview = operations
            .prepare(&workspace, &client, request.clone())
            .await
            .unwrap();
        assert_eq!(
            operations
                .execute(&preview.token, &workspace, &client)
                .await
                .unwrap_err()
                .kind,
            "outcomeUnknown"
        );
        assert!(operations
            .execute(&preview.token, &workspace, &client)
            .await
            .is_err());
        let mut restarted = Operations::new(path.clone());
        assert_eq!(restarted.journal.load().unwrap().len(), 1);
        assert_eq!(
            restarted
                .prepare(&workspace, &client, request)
                .await
                .unwrap_err()
                .kind,
            "pendingOperation"
        );
        assert!(!std::fs::read_to_string(path)
            .unwrap()
            .contains("not persisted secret"));
    }
}
