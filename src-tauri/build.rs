fn main() {
    // Auto-copy MinGW runtime DLL for Windows GNU builds
    #[cfg(all(target_os = "windows", target_env = "gnu"))]
    {
        let dll_name = "libstdc++-6.dll";
        let out_dir = std::path::PathBuf::from(
            std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string()),
        )
        .join(std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string()));

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

    tauri_build::build();
}
