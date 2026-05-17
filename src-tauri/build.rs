fn main() {
    // Auto-copy MinGW runtime DLL for Windows GNU builds
    #[cfg(all(target_os = "windows", target_env = "gnu"))]
    {
        let out_dir = get_out_dir();

        // Copy libstdc++-6.dll from MinGW
        copy_dll("libstdc++-6.dll", &out_dir);
        copy_dll("libgcc_s_seh-1.dll", &out_dir);
        copy_dll("libwinpthread-1.dll", &out_dir);
    }

    // Ensure WebView2Loader.dll is available for portable deployment.
    // Always overwrite from local lib/ cache to prevent stale/incompatible versions.
    #[cfg(target_os = "windows")]
    {
        let out_dir = get_out_dir();
        let target_path = out_dir.join("WebView2Loader.dll");
        let local_cache = std::path::Path::new("lib/WebView2Loader.dll");
        if local_cache.exists() {
            match std::fs::copy(local_cache, &target_path) {
                Ok(_) => println!("cargo:warning=Copied WebView2Loader.dll from lib/"),
                Err(e) => println!("cargo:warning=Failed to copy WebView2Loader.dll: {}", e),
            }
        }
    }

    tauri_build::build();
}

fn get_out_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string()),
    )
    .join(std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string()))
}

#[cfg(all(target_os = "windows", target_env = "gnu"))]
fn copy_dll(dll_name: &str, out_dir: &std::path::Path) {
    let target_path = out_dir.join(dll_name);
    if !target_path.exists() {
        for path in std::env::var("PATH").unwrap_or_default().split(';') {
            let dll_path = std::path::PathBuf::from(path).join(dll_name);
            if dll_path.exists() {
                match std::fs::copy(&dll_path, &target_path) {
                    Ok(_) => {
                        println!("cargo:warning=Copied {} to output", dll_name);
                        break;
                    }
                    Err(e) => println!("cargo:warning=Failed to copy {}: {}", dll_name, e),
                }
            }
        }
    }
}
