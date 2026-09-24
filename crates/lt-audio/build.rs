//! 构建期哨兵：检查内嵌资产就位，缺失时报出自解释指引（否则 rustc 的 include_bytes!
//! 只会报裸路径不存在，不告诉开发者怎么补）。两资产由 uv 管理：pyproject dev 组钉版
//! （onnxruntime + silero-vad），`uv sync` 装进仓库内 .venv（README §二 / assets/SOURCES.md）。
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // build.rs 位于 crates/lt-audio/（两级到仓库根；vad.rs 在 src/ 下是三级，两处不同）
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for rel in [
        ".venv/Lib/site-packages/onnxruntime/capi/onnxruntime.dll",
        ".venv/Lib/site-packages/silero_vad/data/silero_vad.onnx",
    ] {
        let p = root.join(rel);
        if !p.is_file() {
            panic!(
                "内嵌资产缺失：{}\n先跑 uv sync（pyproject dev 组钉版装进仓库内 .venv，详见 README §二）",
                p.display()
            );
        }
        // 跟踪资产本体：资产被删/变更时强制重跑本哨兵与重编译——否则 crate 无变化时
        // cargo 走缓存静默放行（实测盲区），误删 .venv 包不再等到 clean build 才暴露
        println!("cargo:rerun-if-changed={}", p.display());
    }
}
