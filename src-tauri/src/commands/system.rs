/// The release string of the kernel that is running now, e.g. "7.3.1-soplos-bore-ntsync-v4".
#[tauri::command]
pub fn get_running_kernel() -> Result<String, String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("cannot read the running kernel: {e}"))
}
