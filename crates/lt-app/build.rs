//! Windows 资源编译：嵌入 app.ico 与版本信息（单 exe 双击即用，PLAN §1.1）。

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icons/app.ico");
        res.set("FileDescription", "LiveTranslate");
        res.set("ProductName", "LiveTranslate");
        if let Err(e) = res.compile() {
            // 资源失败不阻断构建（icons 缺失场景）
            println!("cargo:warning=winresource 编译失败: {e}");
        }
    }
}
