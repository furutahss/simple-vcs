use crate::{Error, Result, Target, Workspace};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub fn validate(workspace: &Workspace) -> Result<()> {
    if workspace.id.is_empty()
        || workspace.id.len() > 128
        || workspace.name.trim().is_empty()
        || workspace.name.len() > 256
        || workspace.path.len() > 32768
        || workspace.path.contains(['\0', '\n', '\r'])
    {
        return Err(Error::new(
            "invalidInput",
            "表示名・ID・作業コピーパスを確認してください",
        ));
    }
    match &workspace.target {
        Target::Local if !Path::new(&workspace.path).is_absolute() => Err(Error::new(
            "invalidPath",
            "ローカル作業コピーには絶対パスを指定してください",
        )),
        Target::Ssh { host } => {
            if host.is_empty()
                || host.len() > 253
                || host.starts_with('-')
                || !host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err(Error::new(
                    "invalidHost",
                    "SSH configのHost別名を指定してください（ユーザー名・ポートはconfigへ設定）",
                ));
            }
            if !workspace.path.starts_with('/') {
                return Err(Error::new(
                    "invalidPath",
                    "SSH先のLinux絶対パスを指定してください",
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub fn validate_relative(path: &str, windows_paths: bool) -> Result<()> {
    if path.is_empty()
        || path.len() > 32768
        || path.contains('\0')
        || path.starts_with('/')
        || path.split('/').any(|part| part == "..")
        || (windows_paths && path.contains(['\\', ':']))
    {
        return Err(Error::new(
            "invalidPath",
            "作業コピー内の相対パスを指定してください",
        ));
    }
    Ok(())
}

pub fn remote_command(
    workspace: &Workspace,
    args: &[String],
    path: Option<&str>,
) -> Result<String> {
    remote_command_paths(
        workspace,
        if workspace.vcs == crate::VcsKind::Git {
            "git"
        } else {
            "svn"
        },
        args,
        &path.into_iter().collect::<Vec<_>>(),
    )
}

fn remote_command_paths(
    workspace: &Workspace,
    tool: &str,
    args: &[String],
    paths: &[&str],
) -> Result<String> {
    validate(workspace)?;
    let mut guard = String::new();
    for path in paths {
        validate_relative(path, false)?;
        guard.push_str(&format!("root=$(pwd -P) && target=$(realpath -m -- {}) && case \"$target\" in \"$root\"|\"$root\"/*) ;; *) exit 76;; esac && ", quote(path)));
    }
    Ok(format!(
        "cd {} && {guard}LANGUAGE=C LC_ALL=C.UTF-8 {} {}",
        quote(&workspace.path),
        quote(tool),
        args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
    ))
}

pub fn command(
    workspace: &Workspace,
    svn: &str,
    ssh: &str,
    args: &[String],
    path: Option<&str>,
) -> Result<Command> {
    command_with_tool(
        workspace,
        svn,
        ssh,
        args,
        &path.into_iter().collect::<Vec<_>>(),
    )
}

pub fn command_with_tool(
    workspace: &Workspace,
    tool: &str,
    ssh: &str,
    args: &[String],
    paths: &[&str],
) -> Result<Command> {
    command_for_paths(workspace, tool, ssh, args, paths)
}

#[cfg(target_os = "macos")]
fn macos_svn_path(path: &std::ffi::OsStr) -> Result<std::ffi::OsString> {
    // Finder起動ではHomebrewのPATHを継承しない。既存PATHの優先順位は保つ。
    std::env::join_paths(std::env::split_paths(path).chain([
        std::path::PathBuf::from("/opt/homebrew/bin"),
        std::path::PathBuf::from("/usr/local/bin"),
    ]))
    .map_err(|_| Error::new("invalidTool", "CLIの検索パスを構成できません"))
}

pub fn command_for_paths(
    workspace: &Workspace,
    tool: &str,
    ssh: &str,
    args: &[String],
    paths: &[&str],
) -> Result<Command> {
    validate(workspace)?;
    let mut command = match &workspace.target {
        Target::Local => {
            let root = std::fs::canonicalize(&workspace.path)
                .map_err(|_| Error::new("invalidPath", "ローカル作業コピーが見つかりません"))?;
            for path in paths {
                validate_relative(path, cfg!(windows))?;
                let mut candidate = root.join(path);
                while !candidate.exists() && !candidate.is_symlink() {
                    if !candidate.pop() {
                        return Err(Error::new("invalidPath", "対象パスを確認できません"));
                    }
                }
                let canonical = std::fs::canonicalize(candidate)
                    .map_err(|_| Error::new("invalidPath", "対象パスを確認できません"))?;
                if !canonical.starts_with(&root) {
                    return Err(Error::new(
                        "pathOutsideRoot",
                        "作業コピー外へのリンクは参照できません",
                    ));
                }
            }
            let mut command = Command::new(tool);
            #[cfg(target_os = "macos")]
            if tool == "svn" {
                let path = std::env::var_os("PATH")
                    .unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into());
                command.env("PATH", macos_svn_path(&path)?);
            }
            command.current_dir(root).args(args).env("LANGUAGE", "C");
            command
        }
        Target::Ssh { host } => {
            let mut command = Command::new(ssh);
            command.args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                "ForwardAgent=no",
                "-o",
                "ClearAllForwardings=yes",
                "-o",
                "ConnectTimeout=15",
                "--",
                host,
                &remote_command_paths(
                    workspace,
                    if workspace.vcs == crate::VcsKind::Git {
                        "git"
                    } else {
                        "svn"
                    },
                    args,
                    paths,
                )?,
            ]);
            command
        }
    };
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(command)
}

async fn read_bounded(mut stream: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = stream
            .read(&mut buffer)
            .await
            .map_err(|_| Error::new("io", "CLI出力の読取りに失敗しました"))?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > limit {
            return Err(Error::new(
                "outputLimit",
                "出力が表示上限を超えました。対象を絞ってください",
            ));
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

pub async fn execute(mut command: Command, seconds: u64, limit: usize) -> Result<String> {
    command.stdin(Stdio::null());
    String::from_utf8(execute_bytes(command, seconds, limit, None).await?).map_err(|_| {
        Error::new(
            "encoding",
            "CLI出力をUTF-8として解釈できません。ロケールと対象パスを確認してください",
        )
    })
}

pub async fn execute_bytes(
    mut command: Command,
    seconds: u64,
    limit: usize,
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let program = command
        .as_std()
        .get_program()
        .to_string_lossy()
        .into_owned();
    let mut child = command.spawn().map_err(|e| {
        let message = match e.kind() {
            std::io::ErrorKind::NotFound => format!("CLI「{program}」が見つかりません。「CLI設定」に実行ファイルの絶対パスを指定してください。未導入の場合はCLIをインストールしてください。（{e}）"),
            std::io::ErrorKind::PermissionDenied => format!("CLI「{program}」を起動する権限がありません。実行権限とOSのアクセス制限を確認してください。（{e}）"),
            _ => format!("CLI「{program}」を起動できません。「CLI設定」の実行ファイルを確認してください。（{e}）"),
        };
        Error::new(
            if e.kind() == std::io::ErrorKind::NotFound {
                "executableMissing"
            } else {
                "spawn"
            },
            message,
        )
    })?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdin = child.stdin.take();
    let result = tokio::time::timeout(Duration::from_secs(seconds), async {
        tokio::try_join!(
            read_bounded(stdout, limit),
            read_bounded(stderr, 65536),
            async {
                child
                    .wait()
                    .await
                    .map_err(|_| Error::new("io", "CLIの終了確認に失敗しました"))
            },
            async move {
                if let (Some(mut stdin), Some(input)) = (stdin, input) {
                    stdin
                        .write_all(&input)
                        .await
                        .map_err(|_| Error::new("io", "CLIへの入力送信に失敗しました"))?;
                    stdin
                        .shutdown()
                        .await
                        .map_err(|_| Error::new("io", "CLIへの入力終了に失敗しました"))?;
                }
                Ok(())
            }
        )
    })
    .await;
    let (stdout, stderr, status, ()) = match result {
        Ok(Ok(value)) => value,
        result => {
            let _ = child.kill().await;
            return Err(match result {
                Ok(Err(error)) => error,
                _ => Error::new(
                    "timeout",
                    "CLIの応答がタイムアウトしました。SSH先プロセスの停止は保証されません",
                ),
            });
        }
    };
    if !status.success() {
        let kind = if status.code() == Some(76) {
            "pathOutsideRoot"
        } else if status.code() == Some(255) {
            "ssh"
        } else if program == "git" || program.ends_with("/git") {
            "git"
        } else {
            "svn"
        };
        return Err(Error::new(
            kind,
            format!(
                "CLIが失敗しました（終了コード {:?}）: {}",
                status.code(),
                String::from_utf8_lossy(&stderr)
                    .chars()
                    .take(3000)
                    .collect::<String>()
            ),
        ));
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn workspace(host: &str) -> Workspace {
        Workspace {
            id: "test".into(),
            name: "テスト".into(),
            target: Target::Ssh { host: host.into() },
            path: "/srv/project ' 日本語".into(),
            vcs: crate::VcsKind::Svn,
        }
    }

    #[test]
    fn host_options_and_relative_roots_are_rejected() {
        for host in [
            "-oProxyCommand=bad",
            "host name",
            "user@host",
            "a\nwhoami",
            "",
            "$(id)",
        ] {
            assert!(validate(&workspace(host)).is_err(), "{host}");
        }
        assert!(validate(&workspace("team-server.example")).is_ok());
        let mut value = workspace("server");
        value.path = "../project".into();
        assert!(validate(&value).is_err());
    }

    #[test]
    fn target_paths_cannot_escape_or_become_options() {
        for path in [
            "../secret",
            "/etc/passwd",
            "a/../../b",
            "a\0b",
            "C:\\secret",
            "..\\secret",
            "",
        ] {
            assert!(validate_relative(path, true).is_err(), "{path}");
        }
        for path in [
            "日本語.txt",
            "file@name",
            "-option",
            "quote ' space.txt",
            ".",
        ] {
            assert!(validate_relative(path, true).is_ok(), "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn shell_quotes_preserve_bytes_without_executing_substitutions() {
        for value in [
            "",
            "hello ' world",
            "$(printf INJECTED)",
            "a; echo INJECTED",
            "日本語\nnext",
            "\\\"`pwd`",
        ] {
            let output = Command::new("/bin/sh")
                .args(["-c", &format!("printf %s {}", quote(value))])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8(output.stdout).unwrap(), value);
        }
    }

    #[tokio::test]
    async fn missing_binary_is_a_diagnostic_not_empty_success() {
        let result = execute(
            tokio::process::Command::new("simple-vcs-nonexistent-binary"),
            1,
            1024,
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(error.kind, "executableMissing");
        assert!(error.message.contains("simple-vcs-nonexistent-binary"));
        assert!(error.message.contains("絶対パス"));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_svn_search_keeps_user_path_before_homebrew() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("svn");
        std::fs::write(&tool, "#!/bin/sh\nprintf custom-svn").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = tokio::process::Command::new("svn");
        command.env("PATH", macos_svn_path(temp.path().as_os_str()).unwrap());
        assert_eq!(execute(command, 5, 1024).await.unwrap(), "custom-svn");
        let gui_path =
            macos_svn_path(std::ffi::OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin")).unwrap();
        let paths = std::env::split_paths(&gui_path).collect::<Vec<_>>();
        assert!(paths.contains(&std::path::PathBuf::from("/opt/homebrew/bin")));
        assert!(paths.contains(&std::path::PathBuf::from("/usr/local/bin")));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_gui_path_can_launch_installed_svn_without_changing_explicit_tools() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace {
            id: "local".into(),
            name: "テスト".into(),
            target: Target::Local,
            path: temp.path().to_str().unwrap().into(),
            vcs: crate::VcsKind::Svn,
        };
        let args = vec!["--version".into(), "--quiet".into()];
        let command = command(&workspace, "svn", "ssh", &args, None).unwrap();
        assert!(!execute(command, 5, 1024).await.unwrap().trim().is_empty());
        let missing = temp.path().join("explicit-missing-svn");
        let command =
            super::command(&workspace, missing.to_str().unwrap(), "ssh", &args, None).unwrap();
        assert_eq!(
            execute(command, 5, 1024).await.unwrap_err().kind,
            "executableMissing"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_executable_file_reports_program_and_permission_failure() {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("no-permission-svn");
        std::fs::write(&tool, "not executable").unwrap();
        let error = execute(tokio::process::Command::new(&tool), 1, 1024)
            .await
            .unwrap_err();
        assert_eq!(error.kind, "spawn");
        assert!(error.message.contains("no-permission-svn"));
        assert!(error.message.contains("実行権限"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn oversized_output_and_timeout_are_errors() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "printf 123456789"]);
        assert_eq!(
            execute(command, 2, 4).await.unwrap_err().kind,
            "outputLimit"
        );
        let mut command = tokio::process::Command::new("/bin/sleep");
        command.arg("5");
        assert_eq!(execute(command, 1, 1024).await.unwrap_err().kind, "timeout");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdin_and_binary_output_are_drained_without_deadlock() {
        let bytes = vec![0xff; 1024 * 1024];
        let output = execute_bytes(
            tokio::process::Command::new("/bin/cat"),
            5,
            bytes.len(),
            Some(bytes.clone()),
        )
        .await
        .unwrap();
        assert_eq!(output, bytes);
    }
}
