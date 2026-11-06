use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use vcs_core::{Error, Result, Workspace};

fn default_git() -> String {
    "git".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub workspaces: Vec<Workspace>,
    pub svn: String,
    #[serde(default = "default_git")]
    pub git: String,
    pub ssh: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            workspaces: Vec::new(),
            svn: "svn".into(),
            git: "git".into(),
            ssh: "ssh".into(),
        }
    }
}

pub struct Store {
    pub path: PathBuf,
}

pub fn storage_dir(
    app_config: PathBuf,
    executable: &Path,
    appimage: Option<&std::ffi::OsStr>,
) -> PathBuf {
    let portable_dir = appimage
        .and_then(|path| Path::new(path).parent())
        .filter(|path| path.join("PORTABLE").is_file())
        .or_else(|| {
            executable
                .ancestors()
                .take(5)
                .find(|path| path.join("PORTABLE").is_file())
        });
    portable_dir.map(Path::to_path_buf).unwrap_or(app_config)
}

impl Store {
    pub fn load(&self) -> Result<Settings> {
        let Some(bytes) = crate::storage::read(&self.path)? else {
            return Ok(Settings::default());
        };
        let settings: Settings = serde_json::from_slice(&bytes).map_err(|_| Error::new("settings", "設定ファイルが破損しています。内容を確認して復旧してください。自動上書きは行いません"))?;
        validate(&settings)?;
        Ok(settings)
    }
    pub fn save(&self, settings: &Settings) -> Result<()> {
        validate(settings)?;
        let bytes = serde_json::to_vec_pretty(settings)
            .map_err(|_| Error::new("settings", "設定を変換できません"))?;
        if bytes.len() > 1024 * 1024 {
            return Err(Error::new("settings", "設定ファイルが上限を超えています"));
        }
        crate::storage::write(&self.path, &bytes)?;
        Ok(())
    }
}

fn validate(settings: &Settings) -> Result<()> {
    if settings.version != 1 {
        return Err(Error::new(
            "settingsVersion",
            "未対応の設定バージョンです。自動上書きは行いません",
        ));
    }
    for tool in [&settings.svn, &settings.git, &settings.ssh] {
        if tool.trim().is_empty()
            || tool.len() > 32768
            || tool.contains(['\0', '\r', '\n'])
            || tool.starts_with('-')
        {
            return Err(Error::new(
                "invalidTool",
                "実行ファイル名または絶対パスを指定してください",
            ));
        }
    }
    let mut ids = HashSet::new();
    for workspace in &settings.workspaces {
        vcs_core::runner::validate(workspace)?;
        if !ids.insert(&workspace.id) {
            return Err(Error::new(
                "duplicateWorkspace",
                "作業コピーIDが重複しています",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcs_core::Target;

    #[test]
    fn settings_round_trip_and_replacement_preserve_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            path: temp.path().join("nested/settings.json"),
        };
        let mut settings = store.load().unwrap();
        settings.workspaces.push(Workspace {
            id: "one".into(),
            name: "開発".into(),
            target: Target::Local,
            path: temp.path().to_str().unwrap().into(),
            vcs: vcs_core::VcsKind::Svn,
        });
        store.save(&settings).unwrap();
        settings.svn = "svn-custom".into();
        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.svn, "svn-custom");
        assert_eq!(loaded.workspaces[0].name, "開発");
    }

    #[test]
    fn portable_marker_keeps_settings_and_operations_beside_the_app() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("release");
        std::fs::create_dir_all(&app).unwrap();
        let executable = app.join("SimpleVCS.exe");
        let config = temp.path().join("user-config");
        assert_eq!(storage_dir(config.clone(), &executable, None), config);
        std::fs::write(app.join("PORTABLE"), "").unwrap();
        assert_eq!(storage_dir(config, &executable, None), app);
    }

    #[test]
    fn appimage_portable_marker_uses_the_appimage_folder() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("portable");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("PORTABLE"), "").unwrap();
        let appimage = folder.join("simple-vcs.AppImage");
        assert_eq!(
            storage_dir(
                temp.path().join("config"),
                Path::new("/usr/bin/simple-vcs"),
                Some(appimage.as_os_str())
            ),
            folder
        );
    }

    #[test]
    fn corrupt_or_unknown_version_settings_are_not_reset() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            path: temp.path().join("settings.json"),
        };
        for data in [
            "{",
            r#"{"version":2,"workspaces":[],"svn":"svn","ssh":"ssh"}"#,
        ] {
            std::fs::write(&store.path, data).unwrap();
            assert!(store.load().is_err());
            assert_eq!(std::fs::read_to_string(&store.path).unwrap(), data);
        }
    }

    #[test]
    fn invalid_workspace_and_duplicate_ids_are_rejected_before_saving() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            path: temp.path().join("settings.json"),
        };
        let workspace = Workspace {
            id: "same".into(),
            name: "開発".into(),
            target: Target::Ssh {
                host: "-bad".into(),
            },
            path: "/srv/work".into(),
            vcs: vcs_core::VcsKind::Svn,
        };
        let mut settings = Settings {
            workspaces: vec![workspace],
            ..Settings::default()
        };
        assert!(store.save(&settings).is_err());
        settings.workspaces[0].target = Target::Ssh {
            host: "valid".into(),
        };
        settings.workspaces.push(settings.workspaces[0].clone());
        assert!(store.save(&settings).is_err());
        assert!(!store.path.exists());
    }
}
