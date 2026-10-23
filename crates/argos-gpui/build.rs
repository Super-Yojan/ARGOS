fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // rust-lld does not search GCC's private directory, where the unversioned
    // libstdc++ symlink lives. OpenH264 needs that library.
    if let Some(dir) = stdcxx_dir() {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
}

fn stdcxx_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = gcc_print_file_name() {
        return Some(dir);
    }
    let root = std::path::Path::new("/usr/lib/gcc/x86_64-linux-gnu");
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.join("libstdc++.so").exists())
        .collect();
    dirs.sort();
    dirs.pop()
}

fn gcc_print_file_name() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("cc")
        .arg("-print-file-name=libstdc++.so")
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let path = std::path::PathBuf::from(text.trim());
    if path.exists() {
        return path.parent().map(std::path::Path::to_path_buf);
    }
    None
}
