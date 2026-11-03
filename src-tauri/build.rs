fn main() {
    // Declaring the app's commands makes them subject to the ACL: a webview can
    // only call what its capability grants (capabilities/shell.json for the
    // sidebar, capabilities/services.json for the service pages).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "list_services",
            "list_accounts",
            "add_account",
            "remove_account",
            "select_account",
            "reload_account",
            "update_account",
            "reorder_accounts",
            "show_account_menu",
            "show_dnd_menu",
            "set_overlay",
            "update_settings",
            "set_dnd",
            "set_pin",
            "disable_lock",
            "unlock",
            "lock_now",
            "check_update",
            "install_update",
            "open_recent",
            "clear_recent",
            "set_realtime",
            "toggle_pop",
            "close_pop",
            "notify",
        ]),
    ))
    .expect("failed to run tauri-build");
}
