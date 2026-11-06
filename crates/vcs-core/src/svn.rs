use crate::{runner, Error, Result, Target, Workspace};
use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub status: String,
    pub properties: String,
    pub tree_conflicted: bool,
    pub locked: bool,
    pub copied: bool,
    pub switched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub revision: u64,
    pub author: String,
    pub date: String,
    pub message: String,
    pub paths: Vec<LogPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPath {
    pub path: String,
    pub action: String,
}

pub(crate) fn parse<T: serde::de::DeserializeOwned>(xml: &str, root: &[u8]) -> Result<T> {
    let invalid = || {
        Error::new(
            "invalidOutput",
            "SVNのXML出力を解析できません。作業コピー・CLI版・SSH初期化出力を確認してください",
        )
    };
    let mut reader = Reader::from_str(xml);
    let mut depth = 0;
    let mut roots = 0;
    loop {
        match reader.read_event().map_err(|_| invalid())? {
            Event::Start(event) => {
                if depth == 0 {
                    if roots != 0 || event.name().as_ref() != root {
                        return Err(invalid());
                    }
                    roots += 1;
                }
                depth += 1;
            }
            Event::Empty(event) if depth == 0 => {
                if roots != 0 || event.name().as_ref() != root {
                    return Err(invalid());
                }
                roots += 1;
            }
            Event::End(_) => {
                if depth == 0 {
                    return Err(invalid());
                }
                depth -= 1;
            }
            Event::Text(text)
                if depth == 0 && !text.as_ref().iter().all(u8::is_ascii_whitespace) =>
            {
                return Err(invalid())
            }
            Event::DocType(_) => return Err(invalid()),
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || roots != 1 {
        return Err(invalid());
    }
    quick_xml::de::from_str(xml).map_err(|_| invalid())
}

#[derive(Deserialize)]
struct Status {
    target: StatusTarget,
    #[serde(default, rename = "changelist")]
    changelists: Vec<StatusTarget>,
}
#[derive(Deserialize)]
struct StatusTarget {
    #[serde(default, rename = "entry")]
    entries: Vec<StatusEntry>,
}
#[derive(Deserialize)]
struct StatusEntry {
    #[serde(rename = "@path")]
    path: String,
    #[serde(rename = "wc-status")]
    status: WcStatus,
}
#[derive(Deserialize)]
struct WcStatus {
    #[serde(rename = "@item")]
    item: String,
    #[serde(rename = "@props")]
    props: String,
    #[serde(default, rename = "@tree-conflicted")]
    tree_conflicted: bool,
    #[serde(default, rename = "@wc-locked")]
    locked: bool,
    #[serde(default, rename = "@copied")]
    copied: bool,
    #[serde(default, rename = "@switched")]
    switched: bool,
}

pub fn parse_status(xml: &str, windows_paths: bool) -> Result<Vec<Change>> {
    let status: Status = parse(xml, b"status")?;
    status
        .target
        .entries
        .into_iter()
        .chain(status.changelists.into_iter().flat_map(|list| list.entries))
        .map(|entry| {
            let path = if windows_paths {
                entry.path.replace('\\', "/")
            } else {
                entry.path
            };
            let path = path.strip_prefix("./").unwrap_or(&path).to_string();
            runner::validate_relative(&path, windows_paths)?;
            Ok(Change {
                path,
                status: entry.status.item,
                properties: entry.status.props,
                tree_conflicted: entry.status.tree_conflicted,
                locked: entry.status.locked,
                copied: entry.status.copied,
                switched: entry.status.switched,
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct Log {
    #[serde(default, rename = "logentry")]
    entries: Vec<XmlRevision>,
}
#[derive(Deserialize)]
struct XmlRevision {
    #[serde(rename = "@revision")]
    revision: u64,
    #[serde(default)]
    author: String,
    #[serde(default)]
    date: String,
    #[serde(default, rename = "msg")]
    message: String,
    #[serde(default)]
    paths: XmlPaths,
}
#[derive(Default, Deserialize)]
struct XmlPaths {
    #[serde(default, rename = "path")]
    paths: Vec<XmlLogPath>,
}
#[derive(Deserialize)]
struct XmlLogPath {
    #[serde(rename = "@action")]
    action: String,
    #[serde(rename = "$text")]
    path: String,
}

pub fn parse_history(xml: &str) -> Result<Vec<Revision>> {
    let log: Log = parse(xml, b"log")?;
    Ok(log
        .entries
        .into_iter()
        .map(|r| Revision {
            revision: r.revision,
            author: r.author,
            date: r.date,
            message: r.message,
            paths: r
                .paths
                .paths
                .into_iter()
                .map(|p| LogPath {
                    path: p.path,
                    action: p.action,
                })
                .collect(),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub root: String,
    pub url: String,
    pub repository_root: String,
    pub revision: u64,
    pub svn_version: String,
    pub warnings: Vec<String>,
}
#[derive(Deserialize)]
struct XmlInfo {
    entry: InfoEntry,
}
#[derive(Deserialize)]
struct InfoEntry {
    #[serde(rename = "@revision")]
    revision: u64,
    url: String,
    repository: Repository,
    #[serde(rename = "wc-info")]
    wc: WcInfo,
}
#[derive(Deserialize)]
struct Repository {
    root: String,
}
#[derive(Deserialize)]
struct WcInfo {
    #[serde(rename = "wcroot-abspath")]
    root: String,
    #[serde(default)]
    depth: String,
}

pub struct Client {
    pub svn: String,
    pub ssh: String,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            svn: "svn".into(),
            ssh: "ssh".into(),
        }
    }
}

impl Client {
    pub(crate) async fn run(
        &self,
        workspace: &Workspace,
        args: Vec<String>,
        path: Option<&str>,
        limit: usize,
    ) -> Result<String> {
        runner::execute(
            runner::command(workspace, &self.svn, &self.ssh, &args, path)?,
            60,
            limit,
        )
        .await
    }

    pub async fn inspect(&self, workspace: &Workspace) -> Result<Info> {
        let output = self
            .run(
                workspace,
                strings(&["info", "--xml", "--non-interactive", "--", ".@"]),
                None,
                1024 * 1024,
            )
            .await?;
        let info: XmlInfo = parse(&output, b"info")?;
        let version = self
            .run(workspace, strings(&["--version", "--quiet"]), None, 4096)
            .await?;
        let mut warnings = Vec::new();
        if info.entry.wc.depth != "infinity" {
            warnings.push("sparse作業コピーの可能性があります。閲覧のみ対応しています".into());
        }
        let at_root = match workspace.target {
            Target::Local => match (
                std::fs::canonicalize(&workspace.path),
                std::fs::canonicalize(&info.entry.wc.root),
            ) {
                (Ok(configured), Ok(root)) => configured == root,
                _ => false,
            },
            _ => workspace.path.trim_end_matches('/') == info.entry.wc.root.trim_end_matches('/'),
        };
        if !at_root {
            warnings.push(format!(
                "作業コピールートは {} です。ルートを登録し直してください",
                info.entry.wc.root
            ));
        }
        Ok(Info {
            root: info.entry.wc.root,
            url: info.entry.url,
            repository_root: info.entry.repository.root,
            revision: info.entry.revision,
            svn_version: version.trim().into(),
            warnings,
        })
    }

    pub async fn status(&self, workspace: &Workspace) -> Result<Vec<Change>> {
        self.status_with_unchanged(workspace, false).await
    }

    pub async fn status_with_unchanged(
        &self,
        workspace: &Workspace,
        include_unchanged: bool,
    ) -> Result<Vec<Change>> {
        let mut args = strings(&["status", "--xml", "--non-interactive", "--ignore-externals"]);
        if include_unchanged {
            args.push("--verbose".into());
        }
        args.extend(strings(&["--", ".@"]));
        parse_status(
            &self.run(workspace, args, None, 16 * 1024 * 1024).await?,
            cfg!(windows) && workspace.target == Target::Local,
        )
    }

    pub async fn history(
        &self,
        workspace: &Workspace,
        before: Option<u64>,
    ) -> Result<Vec<Revision>> {
        let mut args = strings(&[
            "log",
            "--xml",
            "--verbose",
            "--limit",
            "100",
            "--non-interactive",
        ]);
        if let Some(revision) = before {
            if revision == 0 {
                return Ok(Vec::new());
            }
            args.extend(["-r".into(), format!("{revision}:0")]);
        } else {
            args.extend(strings(&["-r", "HEAD:0"]));
        }
        args.extend(strings(&["--", ".@"]));
        parse_history(&self.run(workspace, args, None, 16 * 1024 * 1024).await?)
    }

    pub async fn latest_history(&self, workspace: &Workspace) -> Result<Vec<Revision>> {
        let info = self.inspect(workspace).await?;
        let mut args = strings(&[
            "log",
            "--xml",
            "--verbose",
            "--limit",
            "10",
            "--non-interactive",
            "-r",
            "HEAD:0",
            "--",
        ]);
        args.push(format!("{}@HEAD", info.url));
        parse_history(&self.run(workspace, args, None, 16 * 1024 * 1024).await?)
    }

    pub async fn diff(
        &self,
        workspace: &Workspace,
        path: &str,
        revision: Option<u64>,
    ) -> Result<String> {
        let mut args = strings(&[
            "diff",
            "--internal-diff",
            "--non-interactive",
            "--depth",
            "empty",
        ]);
        let guarded = if let Some(revision) = revision {
            if revision == 0 {
                return Err(Error::new(
                    "invalidRevision",
                    "リビジョンは1以上を指定してください",
                ));
            }
            let info = self.inspect(workspace).await?;
            let url = historical_url(&info, path)?;
            args.extend([
                "-c".into(),
                revision.to_string(),
                "--".into(),
                format!("{url}@{revision}"),
            ]);
            None
        } else {
            runner::validate_relative(path, cfg!(windows) && workspace.target == Target::Local)?;
            let changes = self.status(workspace).await?;
            let change = changes.iter().find(|c| c.path == path).ok_or_else(|| {
                Error::new(
                    "staleSelection",
                    "対象の状態が変わりました。一覧を再取得してください",
                )
            })?;
            if ["unversioned", "ignored", "external"].contains(&change.status.as_str()) {
                return Err(Error::new(
                    "diffUnavailable",
                    "未追跡ファイル・外部作業コピーにはBASEとの差分がありません",
                ));
            }
            // ローカル差分の対象はpeg構文でなく実パス。末尾@を足すと別名になる。
            args.extend(["--".into(), format!("./{path}")]);
            Some(path)
        };
        let diff = self.run(workspace, args, guarded, 2 * 1024 * 1024).await?;
        if diff.contains('\0') {
            return Err(Error::new("binary", "バイナリの差分は表示できません"));
        }
        Ok(diff)
    }
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}

fn historical_url(info: &Info, path: &str) -> Result<String> {
    if !path.starts_with('/')
        || path.contains('\0')
        || path.split('/').any(|p| p == "." || p == "..")
    {
        return Err(Error::new(
            "invalidPath",
            "履歴のリポジトリ内パスが不正です",
        ));
    }
    let encoded: String = path
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let url = format!("{}{encoded}", info.repository_root.trim_end_matches('/'));
    if url != info.url && !url.starts_with(&format!("{}/", info.url.trim_end_matches('/'))) {
        return Err(Error::new(
            "pathOutsideRoot",
            "この履歴パスは登録した作業コピーの範囲外です",
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_uses_execution_platform_instead_of_gui_platform() {
        let xml = r#"<status><target path="."><entry path="dir\file.txt"><wc-status item="modified" props="none"/></entry><entry path="dir/file.txt"><wc-status item="modified" props="none"/></entry><entry path="name:part.txt"><wc-status item="unversioned" props="none"/></entry></target></status>"#;
        let changes = parse_status(xml, false).unwrap();
        assert_eq!(changes[0].path, "dir\\file.txt");
        assert_eq!(changes[1].path, "dir/file.txt");
        assert_eq!(changes[2].path, "name:part.txt");
        assert!(parse_status(xml, true).is_err());
        let windows = r#"<status><target path="."><entry path=".\dir\file.txt"><wc-status item="modified" props="none"/></entry></target></status>"#;
        assert_eq!(parse_status(windows, true).unwrap()[0].path, "dir/file.txt");
    }

    #[test]
    fn status_keeps_properties_tree_conflicts_and_untracked_paths() {
        let xml = r#"<?xml version="1.0"?><status><target path="."><entry path="日本語 &amp; file@"><wc-status item="modified" props="modified" tree-conflicted="true" wc-locked="true" copied="true" switched="true" revision="1"/></entry><entry path="new.txt"><wc-status item="unversioned" props="none"/></entry></target></status>"#;
        let changes = parse_status(xml, false).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].path, "日本語 & file@");
        assert_eq!(changes[0].properties, "modified");
        assert!(
            changes[0].tree_conflicted
                && changes[0].locked
                && changes[0].copied
                && changes[0].switched
        );
        assert_eq!(changes[1].status, "unversioned");
    }

    #[test]
    fn invalid_xml_and_wrong_root_are_not_clean_workspaces() {
        for xml in [
            "",
            "<status>",
            "<info/>",
            "<status><target path=\".\"><entry path=\"x\"/></target></status>",
        ] {
            assert!(parse_status(xml, false).is_err(), "{xml}");
        }
        assert!(parse_status("<status><target path=\".\"/></status>", false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn history_keeps_multiline_message_and_changed_paths() {
        let entries = parse_history(r#"<log><logentry revision="12"><author>alice</author><date>2026-11-06T01:02:03.000000Z</date><paths><path action="M">/trunk/a</path></paths><msg>one
two &amp; three</msg></logentry><logentry revision="1"><msg/></logentry></log>"#).unwrap();
        assert_eq!(entries[0].revision, 12);
        assert_eq!(entries[0].message, "one\ntwo & three");
        assert_eq!(entries[0].paths[0].path, "/trunk/a");
        assert_eq!(entries[1].author, "");
    }
}
