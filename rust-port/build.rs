// 把官方 VERSION 嵌入 const,供 sqlite3_libversion() 公开 API 返回。
use std::fs;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // 根目录的 sqlite-source/VERSION(由 workflow 复制到此)
    let version_path = Path::new(&manifest)
        .parent()
        .unwrap()
        .join("sqlite-source")
        .join("VERSION");
    let version = fs::read_to_string(&version_path)
        .unwrap_or_else(|_| "0.0.0".to_string())
        .trim()
        .to_string();
    println!("cargo:rustc-env=SQLITE_VERSION={}", version);
    println!("cargo:rerun-if-changed={}", version_path.display());
}
