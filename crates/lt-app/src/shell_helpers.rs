//! W5 下沉三路助手（设备探测/文件对话框/基准转换）——组合根侧的纯函数：
//! UI 不再直持这些执行面（R13/R19/R22），`AppShell` 仅经监督器一次性线程
//! 调用。W7 组织收口：自 shell.rs 拆分，`AppShell` 保持宿主职责（命令路由 +
//! 事件分发 + 停机收尾），本文件是下沉功能的承载面。

use lt_proto::DeviceList;

/// 基准在途标志复位守卫（W5 收口 P3）：RunBench 防线关闭于任何退出路径——
/// 正常收尾与 panic unwind（监督器同时上报）都复位 `bench_active`，杜绝
/// 「线程死了标志卡 true → 后续 RunBench 被永久拒绝」
pub struct BenchActiveGuard(pub std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for BenchActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// 文件对话框形态（导出=保存框 + txt 过滤器；背景图=打开框 + 图片过滤器）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePickKind {
    Export,
    BgImage,
}

/// rfd 同步对话框（W5/R19）：仅允许经 [`super::shell::AppShell::spawn_file_dialog`]
/// 在监督器一次性线程调用——事件循环线程禁止任何同步模态（D-33）。
pub fn pick_file_path(title: &str, default_name: &str, kind: FilePickKind) -> Option<String> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    match kind {
        FilePickKind::Export => {
            dialog = dialog
                .set_file_name(default_name)
                .add_filter("Text", &["txt"]);
            dialog.save_file().map(|p| p.display().to_string())
        }
        FilePickKind::BgImage => dialog
            .add_filter("Images", &["png", "webp", "jpg", "jpeg", "bmp"])
            .pick_file()
            .map(|p| p.display().to_string()),
    }
}

/// 音频设备枚举（W5/R13：自 lt-audio 迁入——UI 帧内 COM 枚举下线；
/// WasapiBackend::new 自带 COM init，独立线程调用安全）
pub fn probe_audio_devices() -> DeviceList {
    #[cfg(windows)]
    {
        use lt_audio::audio::AudioBackend as _;
        let be = lt_audio::audio::wasapi_win::WasapiBackend::new();
        DeviceList {
            outputs: be.list_output_devices().unwrap_or_default(),
            inputs: be.list_input_devices().unwrap_or_default(),
            default_output: be.current_default_output().unwrap_or(None),
        }
    }
    #[cfg(not(windows))]
    {
        DeviceList::default()
    }
}

/// ModelConfig → BenchModel（基准所需连接子集；随基准执行自 lt-ui 迁入——
/// UI 不再直持执行，只发类型化命令载荷）
pub fn to_bench_model(m: &lt_proto::ModelConfig) -> lt_translate::bench::BenchModel {
    lt_translate::bench::BenchModel {
        name: m.name.clone(),
        api_base: m.api_base.clone(),
        api_key: m.api_key.clone(),
        model: m.model.clone(),
        proxy: m.proxy.clone(),
        no_system_role: m.no_system_role,
        // 第二轮评审 ⑫：把该条目的关闭思考配置一并带入——基准测的必须是
        // "生产里实际会发"的请求，否则思考模型上测出的首字延迟是推理起点
        disable_thinking: m.disable_thinking,
        thinking_style: m.thinking_style.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W5f：ModelConfig → BenchModel 基准连接子集转换（随执行自 lt-ui 迁入）
    #[test]
    fn to_bench_model_copies_connection_fields() {
        let cfg = lt_proto::ModelConfig {
            name: "glm".into(),
            api_base: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key: "k".into(),
            model: "glm-4".into(),
            proxy: "system".into(),
            no_system_role: true,
            ..Default::default()
        };
        let b = to_bench_model(&cfg);
        assert_eq!(b.name, "glm");
        assert_eq!(b.api_base, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(b.api_key, "k");
        assert_eq!(b.model, "glm-4");
        assert_eq!(b.proxy, "system");
        assert!(b.no_system_role);
        // 基准之外的字段（价格/overrides/prompt）不参与转换
        let cfg2 = lt_proto::ModelConfig {
            context_turns: 9,
            input_price: 3.0,
            ..cfg
        };
        let b2 = to_bench_model(&cfg2);
        assert_eq!(b2.name, "glm");
        assert_eq!(b2.model, "glm-4");
    }
}
