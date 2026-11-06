mod operations;
mod settings;
mod storage;

use operations::{Operations, PendingOperation, WritePreview};
use serde::Serialize;
use settings::{Settings, Store};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use vcs_core::writes::{WriteRequest, WriteResult};
use vcs_core::{
    svn::{Change, Client, Info, Revision},
    Error, Result, Workspace,
};

struct AppState {
    store: Mutex<Store>,
    operations: tokio::sync::Mutex<Operations>,
    _instance_lock: std::fs::File,
    writing: AtomicBool,
}

struct ActiveWrite<'a>(&'a AtomicBool);
impl Drop for ActiveWrite<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn store(state: &AppState) -> Result<std::sync::MutexGuard<'_, Store>> {
    state.store.lock().map_err(|_| {
        Error::new(
            "settings",
            "設定の排他制御に失敗しました。再起動してください",
        )
    })
}

#[tauri::command]
fn load_workspaces(state: State<'_, AppState>) -> Result<Settings> {
    store(&state)?.load()
}

#[tauri::command]
fn save_workspace(state: State<'_, AppState>, workspace: Workspace) -> Result<Settings> {
    let mut operations = state
        .operations
        .try_lock()
        .map_err(|_| Error::new("busy", "操作中は登録を変更できません"))?;
    vcs_core::runner::validate(&workspace)?;
    let store = store(&state)?;
    let mut settings = store.load()?;
    if let Some(existing) = settings.workspaces.iter().find(|w| w.id == workspace.id) {
        operations.journal.ensure_available(existing)?;
    }
    operations.journal.ensure_available(&workspace)?;
    operations.invalidate_preview();
    if let Some(existing) = settings
        .workspaces
        .iter_mut()
        .find(|w| w.id == workspace.id)
    {
        *existing = workspace;
    } else {
        settings.workspaces.push(workspace);
    }
    store.save(&settings)?;
    Ok(settings)
}

#[tauri::command]
fn remove_workspace(state: State<'_, AppState>, id: String) -> Result<Settings> {
    let mut operations = state
        .operations
        .try_lock()
        .map_err(|_| Error::new("busy", "操作中は登録を変更できません"))?;
    let store = store(&state)?;
    let mut settings = store.load()?;
    if let Some(existing) = settings.workspaces.iter().find(|w| w.id == id) {
        operations.journal.ensure_available(existing)?;
    }
    operations.invalidate_preview();
    settings.workspaces.retain(|workspace| workspace.id != id);
    store.save(&settings)?;
    Ok(settings)
}

#[tauri::command]
fn configure_tools(state: State<'_, AppState>, svn: String, ssh: String) -> Result<Settings> {
    let mut operations = state
        .operations
        .try_lock()
        .map_err(|_| Error::new("busy", "操作中はCLI設定を変更できません"))?;
    operations.invalidate_preview();
    let store = store(&state)?;
    let mut settings = store.load()?;
    settings.svn = svn;
    settings.ssh = ssh;
    store.save(&settings)?;
    Ok(settings)
}

fn saved_client(state: &AppState, id: &str) -> Result<(Workspace, Client)> {
    let settings = store(state)?.load()?;
    let workspace = settings
        .workspaces
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| Error::new("workspaceMissing", "登録された作業コピーが見つかりません"))?;
    Ok((
        workspace,
        Client {
            svn: settings.svn,
            ssh: settings.ssh,
        },
    ))
}

#[tauri::command]
async fn inspect_workspace(state: State<'_, AppState>, id: String) -> Result<Info> {
    let _guard = state.operations.lock().await;
    let (workspace, client) = saved_client(&state, &id)?;
    client.inspect(&workspace).await
}

#[tauri::command]
async fn get_status(
    state: State<'_, AppState>,
    id: String,
    include_unchanged: Option<bool>,
) -> Result<Vec<Change>> {
    let _guard = state.operations.lock().await;
    let (workspace, client) = saved_client(&state, &id)?;
    client
        .status_with_unchanged(&workspace, include_unchanged.unwrap_or(false))
        .await
}

#[tauri::command]
async fn get_diff(
    state: State<'_, AppState>,
    id: String,
    path: String,
    revision: Option<u64>,
) -> Result<String> {
    let _guard = state.operations.lock().await;
    let (workspace, client) = saved_client(&state, &id)?;
    client.diff(&workspace, &path, revision).await
}

#[tauri::command]
async fn get_history(
    state: State<'_, AppState>,
    id: String,
    before: Option<u64>,
) -> Result<Vec<Revision>> {
    let _guard = state.operations.lock().await;
    let (workspace, client) = saved_client(&state, &id)?;
    client.history(&workspace, before).await
}

#[tauri::command]
async fn preview_write(
    state: State<'_, AppState>,
    id: String,
    request: WriteRequest,
) -> Result<WritePreview> {
    let mut operations = state.operations.lock().await;
    let (workspace, client) = saved_client(&state, &id)?;
    operations.prepare(&workspace, &client, request).await
}

#[tauri::command]
async fn execute_write(
    state: State<'_, AppState>,
    id: String,
    token: String,
) -> Result<WriteResult> {
    let mut operations = state.operations.lock().await;
    let (workspace, client) = saved_client(&state, &id)?;
    state.writing.store(true, Ordering::SeqCst);
    let _active_write = ActiveWrite(&state.writing);
    operations.execute(&token, &workspace, &client).await
}

#[tauri::command]
async fn get_pending_operations(state: State<'_, AppState>) -> Result<Vec<PendingOperation>> {
    state.operations.lock().await.journal.load()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryInfo {
    operation: PendingOperation,
    changes: Vec<Change>,
    history: Vec<Revision>,
}

#[tauri::command]
async fn inspect_pending_operation(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<RecoveryInfo> {
    let operations = state.operations.lock().await;
    let operation = operations
        .journal
        .load()?
        .into_iter()
        .find(|p| p.id == operation_id)
        .ok_or_else(|| Error::new("operationMissing", "未完了操作が見つかりません"))?;
    let settings = store(&state)?.load()?;
    let client = Client {
        svn: settings.svn,
        ssh: settings.ssh,
    };
    let changes = client.status(&operation.workspace).await?;
    let history = client.latest_history(&operation.workspace).await?;
    Ok(RecoveryInfo {
        operation,
        changes,
        history,
    })
}

#[tauri::command]
async fn acknowledge_pending_operation(
    state: State<'_, AppState>,
    operation_id: String,
    confirmed: bool,
) -> Result<()> {
    state
        .operations
        .lock()
        .await
        .journal
        .acknowledge(&operation_id, confirmed)
}

#[tauri::command]
async fn choose_directory(app: tauri::AppHandle) -> Result<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map_err(|_| Error::new("invalidPath", "フォルダのパスを取得できません"))?
                    .to_str()
                    .map(str::to_string)
                    .ok_or_else(|| Error::new("encoding", "UTF-8で表せないパスは指定できません"))
            })
            .transpose()
    })
    .await
    .map_err(|_| Error::new("dialog", "フォルダ選択に失敗しました"))?
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config)?;
            let instance_lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(config.join("instance.lock"))?;
            instance_lock.try_lock().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "Simple VCSは既に起動しています。先に起動したアプリを終了してください",
                )
            })?;
            app.manage(AppState {
                store: Mutex::new(Store {
                    path: config.join("settings.json"),
                }),
                operations: tokio::sync::Mutex::new(Operations::new(
                    config.join("operations.json"),
                )),
                _instance_lock: instance_lock,
                writing: AtomicBool::new(false),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.state::<AppState>().writing.load(Ordering::SeqCst) {
                    api.prevent_close();
                    window.app_handle().dialog().message("SVN操作の結果を確認しています。処理が終了するまでお待ちください。強制終了した場合は、次回起動時に操作の成否を確認してください。").title("SVN操作中").show(|_| {});
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_workspaces,
            save_workspace,
            remove_workspace,
            configure_tools,
            inspect_workspace,
            get_status,
            get_diff,
            get_history,
            preview_write,
            execute_write,
            get_pending_operations,
            inspect_pending_operation,
            acknowledge_pending_operation,
            choose_directory
        ])
        .build(tauri::generate_context!())
        .expect("Simple VCS failed to start")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if app.state::<AppState>().writing.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    app.dialog().message("SVN操作中は終了できません。結果が返るまでお待ちください。強制終了時は次回起動時に成否確認が必要です。").title("SVN操作中").show(|_| {});
                }
            }
        });
}
