use std::{fs, path::Path, process::Command};
use vcs_core::writes::{WriteAction, WriteRequest};
use vcs_core::{runner, svn::Client, Target, Workspace};

fn request(action: WriteAction, paths: &[&str]) -> WriteRequest {
    WriteRequest {
        action,
        paths: paths.iter().map(|p| p.to_string()).collect(),
        message: "日本語の記録\n$(echo literal) ' @".into(),
    }
}

fn svn(directory: &Path, args: &[&str]) {
    let output = Command::new("svn")
        .current_dir(directory)
        .args(args)
        .output()
        .expect("SVN CLIが必要です");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Fixture {
    _temp: tempfile::TempDir,
    workspace: Workspace,
}

#[tokio::test]
async fn commit_only_selected_change_and_preserve_multiline_message() {
    let f = Fixture::new();
    fs::write(Path::new(&f.workspace.path).join("second.txt"), "second").unwrap();
    svn(Path::new(&f.workspace.path), &["add", "second.txt"]);
    let client = Client::default();
    let plan = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Commit, &["-日本語@ file.txt"]),
        )
        .await
        .unwrap();
    client.verify_write(&plan).await.unwrap();
    let result = client.execute_write(&plan).await.unwrap();
    assert_eq!(result.revision, Some(2));
    let changes = client.status(&f.workspace).await.unwrap();
    assert!(!changes.iter().any(|c| c.path == "-日本語@ file.txt"));
    assert!(changes
        .iter()
        .any(|c| c.path == "second.txt" && c.status == "added"));
    assert_eq!(
        client.history(&f.workspace, None).await.unwrap()[0].revision,
        2,
        "作業コピーのルートを更新しなくても最新コミットを表示する"
    );
    svn(Path::new(&f.workspace.path), &["update"]);
    assert_eq!(
        client.history(&f.workspace, None).await.unwrap()[0].message,
        "日本語の記録\n$(echo literal) ' @"
    );
}

#[tokio::test]
async fn changed_content_after_preview_requires_new_confirmation() {
    let f = Fixture::new();
    let client = Client::default();
    let plan = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Revert, &["-日本語@ file.txt"]),
        )
        .await
        .unwrap();
    fs::write(
        Path::new(&f.workspace.path).join("-日本語@ file.txt"),
        "changed again\n",
    )
    .unwrap();
    assert_eq!(
        client.verify_write(&plan).await.unwrap_err().kind,
        "stalePreview"
    );
}

#[tokio::test]
async fn add_directory_and_commit_parent_do_not_include_unselected_children() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    fs::create_dir(root.join("folder")).unwrap();
    fs::write(root.join("folder/selected.txt"), "selected").unwrap();
    fs::write(root.join("folder/other.txt"), "other").unwrap();
    let client = Client::default();
    let add = client
        .prepare_write(&f.workspace, request(WriteAction::Add, &["folder"]))
        .await
        .unwrap();
    client.execute_write(&add).await.unwrap();
    let changes = client.status(&f.workspace).await.unwrap();
    assert!(changes
        .iter()
        .any(|c| c.path == "folder/other.txt" && c.status == "unversioned"));
    let add = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Add, &["folder/selected.txt"]),
        )
        .await
        .unwrap();
    client.execute_write(&add).await.unwrap();
    assert!(client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Commit, &["folder/selected.txt"])
        )
        .await
        .is_err());
    let commit = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Commit, &["folder", "folder/selected.txt"]),
        )
        .await
        .unwrap();
    client.execute_write(&commit).await.unwrap();
    assert!(client
        .status(&f.workspace)
        .await
        .unwrap()
        .iter()
        .any(|c| c.path == "folder/other.txt" && c.status == "unversioned"));
}

#[tokio::test]
async fn revert_selected_file_leaves_untracked_file_and_other_changes() {
    let f = Fixture::new();
    let client = Client::default();
    let plan = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Revert, &["-日本語@ file.txt"]),
        )
        .await
        .unwrap();
    client.execute_write(&plan).await.unwrap();
    assert_eq!(
        fs::read_to_string(Path::new(&f.workspace.path).join("-日本語@ file.txt")).unwrap(),
        "old\n"
    );
    assert!(Path::new(&f.workspace.path).join("untracked.txt").exists());
    assert!(client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Revert, &["untracked.txt"])
        )
        .await
        .is_err());
}

#[tokio::test]
async fn delete_rejects_modified_file_and_schedules_clean_file() {
    let f = Fixture::new();
    let client = Client::default();
    assert!(client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Delete, &["-日本語@ file.txt"])
        )
        .await
        .is_err());
    svn(
        Path::new(&f.workspace.path),
        &["revert", "--", "-日本語@ file.txt@"],
    );
    let plan = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Delete, &["-日本語@ file.txt"]),
        )
        .await
        .unwrap();
    client.execute_write(&plan).await.unwrap();
    assert!(!Path::new(&f.workspace.path)
        .join("-日本語@ file.txt")
        .exists());
    assert!(client
        .status(&f.workspace)
        .await
        .unwrap()
        .iter()
        .any(|c| c.path == "-日本語@ file.txt" && c.status == "deleted"));
}

#[tokio::test]
async fn whole_workspace_update_and_cleanup_work_without_deleting_untracked() {
    let f = Fixture::new();
    let client = Client::default();
    for action in [WriteAction::Cleanup, WriteAction::Update] {
        let plan = client
            .prepare_write(&f.workspace, request(action, &[]))
            .await
            .unwrap();
        client.execute_write(&plan).await.unwrap();
    }
    assert!(Path::new(&f.workspace.path).join("untracked.txt").exists());
}

#[tokio::test]
async fn empty_selection_and_empty_commit_message_are_rejected() {
    let f = Fixture::new();
    let client = Client::default();
    assert!(client
        .prepare_write(&f.workspace, request(WriteAction::Revert, &[]))
        .await
        .is_err());
    let mut empty = request(WriteAction::Commit, &["-日本語@ file.txt"]);
    empty.message.clear();
    assert!(client.prepare_write(&f.workspace, empty).await.is_err());
}

#[tokio::test]
async fn update_postpones_conflicts_and_resolve_keeps_external_edit() {
    let f = Fixture::new();
    let peer = f._temp.path().join("peer");
    let repo = f
        ._temp
        .path()
        .join("repo")
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let url = format!(
        "file://{}{repo}",
        if repo.starts_with('/') { "" } else { "/" }
    );
    svn(f._temp.path(), &["checkout", &url, peer.to_str().unwrap()]);
    fs::write(peer.join("-日本語@ file.txt"), "server\n").unwrap();
    svn(&peer, &["commit", "-m", "server change"]);
    let client = Client::default();
    let update = client
        .prepare_write(&f.workspace, request(WriteAction::Update, &[]))
        .await
        .unwrap();
    client.execute_write(&update).await.unwrap();
    assert!(client
        .status(&f.workspace)
        .await
        .unwrap()
        .iter()
        .any(|c| c.status == "conflicted"));
    assert!(client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Commit, &["-日本語@ file.txt"])
        )
        .await
        .is_err());
    fs::write(
        Path::new(&f.workspace.path).join("-日本語@ file.txt"),
        "resolved outside\n",
    )
    .unwrap();
    let resolve = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Resolve, &["-日本語@ file.txt"]),
        )
        .await
        .unwrap();
    client.execute_write(&resolve).await.unwrap();
    assert!(!client
        .status(&f.workspace)
        .await
        .unwrap()
        .iter()
        .any(|c| c.status == "conflicted"));
    assert_eq!(
        fs::read_to_string(Path::new(&f.workspace.path).join("-日本語@ file.txt")).unwrap(),
        "resolved outside\n"
    );
}

#[tokio::test]
async fn externals_and_copy_operations_block_writes() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    let client = Client::default();
    svn(root, &["propset", "svn:externals", "^/ external-work", "."]);
    assert_eq!(
        client
            .prepare_write(
                &f.workspace,
                request(WriteAction::Commit, &["-日本語@ file.txt"])
            )
            .await
            .unwrap_err()
            .kind,
        "unsupportedWorkspace"
    );
    svn(root, &["propdel", "svn:externals", "."]);
    svn(root, &["copy", "--", "-日本語@ file.txt@", "copy.txt"]);
    assert_eq!(
        client
            .prepare_write(&f.workspace, request(WriteAction::Commit, &["copy.txt"]))
            .await
            .unwrap_err()
            .kind,
        "unsupportedWorkspace"
    );
}

#[tokio::test]
async fn directory_delete_refuses_ignored_data_and_partial_delete_commit() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    fs::create_dir(root.join("dir")).unwrap();
    fs::write(root.join("dir/a.txt"), "a").unwrap();
    svn(root, &["add", "dir"]);
    svn(root, &["propset", "svn:ignore", "*.tmp", "dir"]);
    svn(root, &["commit", "-m", "directory", "--", "dir@"]);
    fs::write(root.join("dir/private.tmp"), "keep me").unwrap();
    let client = Client::default();
    assert!(client
        .prepare_write(&f.workspace, request(WriteAction::Delete, &["dir"]))
        .await
        .is_err());
    assert!(root.join("dir/private.tmp").exists());
    fs::remove_file(root.join("dir/private.tmp")).unwrap();
    let deletion = client
        .prepare_write(&f.workspace, request(WriteAction::Delete, &["dir"]))
        .await
        .unwrap();
    assert!(deletion.paths.iter().any(|p| p == "dir/a.txt"));
    client.execute_write(&deletion).await.unwrap();
    assert!(client
        .prepare_write(&f.workspace, request(WriteAction::Commit, &["dir"]))
        .await
        .is_err());
    let commit = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Commit, &["dir", "dir/a.txt"]),
        )
        .await
        .unwrap();
    client.execute_write(&commit).await.unwrap();
}

#[tokio::test]
async fn revert_deleted_directory_restores_selected_children() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    fs::create_dir(root.join("dir")).unwrap();
    fs::write(root.join("dir/a.txt"), "restore me").unwrap();
    svn(root, &["add", "dir"]);
    svn(root, &["commit", "-m", "directory", "dir"]);
    svn(root, &["delete", "dir"]);
    let client = Client::default();
    let plan = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Revert, &["dir", "dir/a.txt"]),
        )
        .await
        .unwrap();
    client.execute_write(&plan).await.unwrap();
    assert_eq!(
        fs::read_to_string(root.join("dir/a.txt")).unwrap(),
        "restore me"
    );
    assert!(!client
        .status(&f.workspace)
        .await
        .unwrap()
        .iter()
        .any(|c| c.path == "dir/a.txt"));
}

#[tokio::test]
async fn reverting_deleted_child_requires_deleted_parent_to_be_selected() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    fs::create_dir(root.join("dir")).unwrap();
    fs::write(root.join("dir/a.txt"), "restore me").unwrap();
    svn(root, &["add", "dir"]);
    svn(root, &["commit", "-m", "directory", "dir"]);
    svn(root, &["delete", "dir"]);
    assert_eq!(
        Client::default()
            .prepare_write(&f.workspace, request(WriteAction::Revert, &["dir/a.txt"]))
            .await
            .unwrap_err()
            .kind,
        "parentRequired"
    );
}

#[tokio::test]
async fn unchanged_files_can_be_listed_for_deletion_selection() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    svn(root, &["revert", "--", "-日本語@ file.txt@"]);
    let changes = Client::default()
        .status_with_unchanged(&f.workspace, true)
        .await
        .unwrap();
    assert!(changes
        .iter()
        .any(|c| c.path == "-日本語@ file.txt" && c.status == "normal"));
}

#[tokio::test]
async fn reverting_added_directory_keeps_files_on_disk() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    fs::create_dir(root.join("dir")).unwrap();
    fs::write(root.join("dir/a.txt"), "keep me").unwrap();
    svn(root, &["add", "dir"]);
    let client = Client::default();
    assert!(client
        .prepare_write(&f.workspace, request(WriteAction::Revert, &["dir"]))
        .await
        .is_err());
    let plan = client
        .prepare_write(
            &f.workspace,
            request(WriteAction::Revert, &["dir", "dir/a.txt"]),
        )
        .await
        .unwrap();
    client.execute_write(&plan).await.unwrap();
    assert_eq!(
        fs::read_to_string(root.join("dir/a.txt")).unwrap(),
        "keep me"
    );
    assert!(client
        .status(&f.workspace)
        .await
        .unwrap()
        .iter()
        .any(|c| c.path == "dir" && c.status == "unversioned"));
}

#[tokio::test]
async fn sparse_child_directory_blocks_write_at_normal_root() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    fs::create_dir(root.join("dir")).unwrap();
    fs::write(root.join("dir/a.txt"), "a").unwrap();
    svn(root, &["add", "dir"]);
    svn(root, &["commit", "-m", "directory", "dir"]);
    svn(root, &["update", "--set-depth", "empty", "dir"]);
    assert_eq!(
        Client::default()
            .prepare_write(
                &f.workspace,
                request(WriteAction::Commit, &["-日本語@ file.txt"])
            )
            .await
            .unwrap_err()
            .kind,
        "unsupportedWorkspace"
    );
}

#[tokio::test]
async fn expanded_directory_limit_prevents_delete_but_not_clean_workspace_update() {
    let f = Fixture::new();
    let root = Path::new(&f.workspace.path);
    fs::create_dir(root.join("bulk")).unwrap();
    for index in 0..201 {
        fs::write(
            root.join(format!("bulk/{index:03}-{}.txt", "x".repeat(80))),
            "keep",
        )
        .unwrap();
    }
    svn(root, &["add", "bulk"]);
    svn(root, &["commit", "-m", "seed bulk files", "bulk"]);
    let client = Client::default();
    assert!(
        client
            .prepare_write(&f.workspace, request(WriteAction::Delete, &["bulk"]))
            .await
            .is_err(),
        "展開後に上限を超える削除を開始しない"
    );
    assert!(root.join("bulk").exists());
    assert!(
        client
            .prepare_write(&f.workspace, request(WriteAction::Update, &[]))
            .await
            .is_ok(),
        "変更のないファイル数で更新を拒否しない"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn svn_accepts_utf8_commit_message_from_stdin_without_shell_interpolation() {
    let f = Fixture::new();
    let args: Vec<String> = [
        "commit",
        "--file",
        "/dev/stdin",
        "--encoding",
        "UTF-8",
        "--non-interactive",
        "--",
        "./-日本語@ file.txt@",
    ]
    .iter()
    .map(|s| (*s).into())
    .collect();
    let command =
        runner::command(&f.workspace, "svn", "ssh", &args, Some("-日本語@ file.txt")).unwrap();
    let input = "標準入力からの記録\n$(literal) 'quoted'";
    runner::execute_bytes(command, 10, 1024 * 1024, Some(input.as_bytes().to_vec()))
        .await
        .unwrap();
    assert_eq!(
        Client::default()
            .latest_history(&f.workspace)
            .await
            .unwrap()[0]
            .message,
        input
    );
}

#[cfg(unix)]
#[tokio::test]
async fn nonzero_write_exit_is_never_a_retryable_success() {
    let f = Fixture::new();
    let plan = Client::default()
        .prepare_write(
            &f.workspace,
            request(WriteAction::Revert, &["-日本語@ file.txt"]),
        )
        .await
        .unwrap();
    let failed = Client {
        svn: "/usr/bin/false".into(),
        ssh: "ssh".into(),
    };
    assert_eq!(
        failed.execute_write(&plan).await.unwrap_err().kind,
        "outcomeUnknown"
    );
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let output = Command::new("svnadmin")
            .args(["create", repo.to_str().unwrap()])
            .output()
            .expect("svnadminが必要です");
        assert!(output.status.success());
        let work = temp.path().join("work ' 日本語 $(printf unsafe)");
        let repo_path = repo.to_str().unwrap().replace('\\', "/");
        let repo_url = format!(
            "file://{}{repo_path}",
            if repo_path.starts_with('/') { "" } else { "/" }
        );
        svn(
            temp.path(),
            &["checkout", &repo_url, work.to_str().unwrap()],
        );
        fs::write(work.join("-日本語@ file.txt"), "old\n").unwrap();
        svn(&work, &["add", "--", "-日本語@ file.txt@"]);
        svn(&work, &["commit", "-m", "最初の記録\n二行目"]);
        svn(&work, &["update"]);
        fs::write(work.join("-日本語@ file.txt"), "new\n").unwrap();
        fs::write(work.join("untracked.txt"), "untracked").unwrap();
        Self {
            workspace: Workspace {
                id: "integration".into(),
                name: "統合テスト".into(),
                target: Target::Local,
                path: work.to_str().unwrap().into(),
                vcs: vcs_core::VcsKind::Svn,
            },
            _temp: temp,
        }
    }
}

#[tokio::test]
async fn real_svn_reads_changes_diff_history_and_properties() {
    let fixture = Fixture::new();
    let client = Client::default();
    let info = client.inspect(&fixture.workspace).await.unwrap();
    assert_eq!(info.revision, 1);
    assert!(info.warnings.is_empty(), "{:?}", info.warnings);
    let changes = client.status(&fixture.workspace).await.unwrap();
    assert!(changes
        .iter()
        .any(|c| c.path == "-日本語@ file.txt" && c.status == "modified"));
    assert!(changes
        .iter()
        .any(|c| c.path == "untracked.txt" && c.status == "unversioned"));
    let diff = client
        .diff(&fixture.workspace, "-日本語@ file.txt", None)
        .await
        .unwrap();
    assert!(diff.contains("-old") && diff.contains("+new"), "{diff}");
    assert!(client
        .diff(&fixture.workspace, "untracked.txt", None)
        .await
        .is_err());
    let history = client.history(&fixture.workspace, None).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].message, "最初の記録\n二行目");
    let old = client
        .diff(&fixture.workspace, "/-日本語@ file.txt", Some(1))
        .await
        .unwrap();
    assert!(old.contains("+old"));
    svn(
        Path::new(&fixture.workspace.path),
        &["propset", "test:note", "changed", "."],
    );
    assert!(client
        .status(&fixture.workspace)
        .await
        .unwrap()
        .iter()
        .any(|c| c.path == "." && c.properties == "modified"));
    let properties = client.diff(&fixture.workspace, ".", None).await.unwrap();
    assert!(properties.contains("test:note"));
    assert!(
        !properties.contains("+new"),
        "ディレクトリ選択は配下のファイル差分を含めない"
    );
}

#[tokio::test]
async fn changelist_members_are_not_reported_as_clean() {
    let fixture = Fixture::new();
    svn(
        Path::new(&fixture.workspace.path),
        &["changelist", "work", "--", "-日本語@ file.txt@"],
    );
    let client = Client::default();
    let changes = client.status(&fixture.workspace).await.unwrap();
    assert!(changes
        .iter()
        .any(|c| c.path == "-日本語@ file.txt" && c.status == "modified"));
    assert!(client
        .diff(&fixture.workspace, "-日本語@ file.txt", None)
        .await
        .unwrap()
        .contains("+new"));
}

#[cfg(unix)]
#[tokio::test]
async fn linux_style_names_keep_backslashes_and_colons_as_literal_bytes() {
    let fixture = Fixture::new();
    let root = Path::new(&fixture.workspace.path);
    fs::write(root.join("literal\\name:part.txt"), "before\n").unwrap();
    svn(root, &["add", "--", "literal\\name:part.txt@"]);
    svn(root, &["commit", "-m", "literal path"]);
    fs::write(root.join("literal\\name:part.txt"), "after\n").unwrap();
    let client = Client::default();
    let changes = client.status(&fixture.workspace).await.unwrap();
    assert!(changes.iter().any(|c| c.path == "literal\\name:part.txt"));
    let diff = client
        .diff(&fixture.workspace, "literal\\name:part.txt", None)
        .await
        .unwrap();
    assert!(diff.contains("+after"));
    let historical = client
        .diff(&fixture.workspace, "/literal\\name:part.txt", Some(2))
        .await
        .unwrap();
    assert!(historical.contains("+before"));
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_and_parent_traversal_cannot_read_outside_workspace() {
    let fixture = Fixture::new();
    let outside = fixture._temp.path().join("outside.txt");
    fs::write(&outside, "secret").unwrap();
    std::os::unix::fs::symlink(outside, Path::new(&fixture.workspace.path).join("link")).unwrap();
    svn(Path::new(&fixture.workspace.path), &["add", "link"]);
    let error = Client::default()
        .diff(&fixture.workspace, "link", None)
        .await
        .unwrap_err();
    assert_eq!(error.kind, "pathOutsideRoot");
    assert!(Client::default()
        .diff(&fixture.workspace, "../outside.txt", None)
        .await
        .is_err());
}

#[cfg(unix)]
#[test]
fn remote_shell_command_preserves_paths_and_enforces_boundary() {
    let fixture = Fixture::new();
    let args: Vec<String> = [
        "diff",
        "--internal-diff",
        "--non-interactive",
        "--",
        "./-日本語@ file.txt",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let mut remote = fixture.workspace.clone();
    remote.target = Target::Ssh {
        host: "test-host".into(),
    };
    let guarded = if cfg!(target_os = "linux") {
        Some("-日本語@ file.txt")
    } else {
        None
    };
    let command = runner::remote_command(&remote, &args, guarded).unwrap();
    // macOSではC.UTF-8がないため、シェル引用の検証のみ既存ロケールで実行する。
    let output = Command::new("/bin/sh")
        .args(["-c", &command.replace("LC_ALL=C.UTF-8 ", "")])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout).unwrap().contains("+new"));
}
