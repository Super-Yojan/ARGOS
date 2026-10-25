fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Release packages set ARGOS_VPX_STATIC to the folder holding libvpx.a
    // (vpx.lib on Windows), so the download does not need a libvpx with the
    // same soname as the build machine.
    println!("cargo:rerun-if-env-changed=ARGOS_VPX_STATIC");
    if let Some(dir) = std::env::var_os("ARGOS_VPX_STATIC").filter(|dir| !dir.is_empty()) {
        println!(
            "cargo:rustc-link-search=native={}",
            std::path::Path::new(&dir).display()
        );
        println!("cargo:rustc-link-lib=static=vpx");
    } else {
        println!("cargo:rustc-link-lib=vpx");
    }
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
