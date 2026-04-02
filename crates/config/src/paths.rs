use std::path::PathBuf;

/// Returns ~/.ccode/
pub fn ccode_dir() -> PathBuf {
    dirs_home().join(".ccode")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
