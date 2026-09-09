//! Windows 资源编译：嵌入 app.ico、版本信息与版权（单 exe 双击即用，
//! PLAN §1.1 + WD-2 版本可见性：资源管理器「属性 → 详细信息」可见版本）。

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = env!("CARGO_PKG_VERSION");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icons/app.ico");
        res.set("FileDescription", "LiveTranslate");
        res.set("ProductName", "LiveTranslate");
        // WD-2：版本四元组（FileVersion 需 x.y.z.w，.w 缺省补 0）
        let v4 = format!("{version}.0");
        res.set("FileVersion", &v4);
        res.set("ProductVersion", &v4);
        res.set("OriginalFilename", "livetranslate.exe");
        res.set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            // 资源失败不阻断构建（icons 缺失场景）
            println!("cargo:warning=winresource 编译失败: {e}");
        }
    }
}
