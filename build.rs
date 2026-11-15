fn main() {
    println!("cargo:rerun-if-changed=assets/ndm.ico");

    // 只有交叉/原生编译到 Windows 时才嵌入 exe 图标与版本信息。
    // 找不到 rc.exe / windres 时优雅跳过，避免影响 cargo check。
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target != "windows" {
        return;
    }
    if !std::path::Path::new("assets/ndm.ico").exists() {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/ndm.ico");
    res.set("ProductName", "NDM-RS");
    res.set("FileDescription", "NDM-RS Download Manager");
    res.set("CompanyName", "NDM-RS");
    res.set("LegalCopyright", "MIT License");
    res.set("FileVersion", "0.1.0.0");
    res.set("ProductVersion", "0.1.0.0");
    if let Err(e) = res.compile() {
        println!("cargo:warning=skip windows icon resource: {e}");
    }
}
