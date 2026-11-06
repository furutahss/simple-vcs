fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "load_workspaces",
            "save_workspace",
            "remove_workspace",
            "configure_tools",
            "inspect_workspace",
            "get_status",
            "get_diff",
            "get_history",
            "preview_write",
            "execute_write",
            "get_pending_operations",
            "inspect_pending_operation",
            "acknowledge_pending_operation",
            "choose_directory",
            "get_git_info",
            "get_git_status",
            "get_git_diff",
            "get_git_history",
            "get_git_commit_diff",
            "get_git_branches",
            "git_write",
        ]),
    ))
    .expect("Tauri build configuration failed");
}
