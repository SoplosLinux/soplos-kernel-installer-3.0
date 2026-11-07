mod bootloader;
mod build;
mod commands;
mod history;
mod limine;
mod kernel_sources;
mod privileged;
mod secure_boot_ops;
mod signer;
mod system_theme;

pub use privileged::run_ops_file;
pub use secure_boot_ops::sign_boot;
pub use build::install::patch_nvidia_after_install;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin registered: it needs to intercept and
        // exit a second launch before anything else runs. Two windows must
        // never be able to build or install a kernel at the same time.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            system_theme::get_system_theme,
            commands::system::get_running_kernel,
            commands::hardware::get_hardware_info,
            commands::kernel_build::list_kernel_versions,
            commands::kernel_build::get_build_environment,
            commands::kernel_build::start_build,
            commands::kernel_build::cancel_build,
            commands::kernel_build::get_build_job,
            commands::kernel_build::get_build_sources,
            commands::kernel_build::get_batch_queue,
            commands::kernel_build::start_batch,
            commands::kernel_build::batch_built,
            commands::kernel_build::clean_build_files,
            commands::secure_boot::get_secure_boot_state,
            commands::secure_boot::sb_generate_keys,
            commands::secure_boot::sb_install_tools,
            commands::secure_boot::sb_enroll,
            commands::secure_boot::sb_delete_key,
            commands::secure_boot::sb_reset,
            commands::secure_boot::sb_sign,
            commands::secure_boot::sb_sign_boot,
            commands::secure_boot::sb_limine_refresh,
            commands::secure_boot::sb_export_cert,
            commands::secure_boot::sb_set_auto_sign,
            commands::secure_boot::reboot_now,
            commands::other_kernels::get_other_kernels,
            commands::other_kernels::debian_kernel_action,
            commands::other_kernels::xanmod_action,
            commands::other_kernels::liquorix_action,
            commands::installed::list_installed_kernels,
            commands::installed::remove_installed_kernel,
            commands::soplos_kernels::get_repo_state,
            commands::soplos_kernels::repo_add,
            commands::soplos_kernels::repo_remove,
            commands::soplos_kernels::repo_refresh,
            commands::soplos_kernels::kernel_package_action,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Soplos Kernel Installer");
}
