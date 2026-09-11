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

/// 取消请求是否命中在途探测（D-85 纯判据）：号不符即忽略——过期的取消请求
/// 不得误伤随后启动的新探测（"中断后立刻重测"场景）。
pub fn probe_cancel_matches(in_flight: u64, requested: u64) -> bool {
    in_flight != 0 && in_flight == requested
}

/// 开始一次新探测的槽位簿记（D-85 §4.7 取代语义，2026-09-11 评审修复提取为
/// 纯函数以便直接测试）：
/// - 已有在途（`in_flight != 0`）→ **先置位旧探测的取消标志**（取代而非拒绝：
///   旧线程退出可滞后一个轮询周期，拒绝会把"中断后立刻重测"变成空等）；
/// - 换上新取消标志并把在途号改为 `new_id`。
///
/// 返回新探测要持有的取消标志。注意 `new_id == 0` 是"无在途"哨兵——UI 号源
/// 从 1 起（`ProbeUiState::begin`），本函数不再为哨兵做例外。
pub fn begin_probe_slot(
    in_flight: &std::sync::atomic::AtomicU64,
    cancel: &mut std::sync::Arc<std::sync::atomic::AtomicBool>,
    new_id: u64,
) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    use std::sync::atomic::Ordering;
    debug_assert!(new_id != 0, "探测号不得为 0（0 是'无在途'哨兵）");
    let old = in_flight.load(Ordering::SeqCst);
    if old != 0 {
        cancel.store(true, Ordering::SeqCst);
        tracing::warn!("连接测试被新请求取代：#{old} → #{new_id}");
    }
    let fresh = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    *cancel = fresh.clone();
    in_flight.store(new_id, Ordering::SeqCst);
    fresh
}

/// 连接测试线程的退出守卫（D-85）：**按号条件复位**在途号——旧探测被新请求
/// 取代后退出时，不得把新探测在途号抹掉（否则随后的「中断」会被判为过期而失效）。
/// 与 [`BenchActiveGuard`] 同款纪律：任何退出路径（含 panic unwind）都复位。
pub struct ProbeExitGuard {
    pub id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub probe_id: u64,
}

impl Drop for ProbeExitGuard {
    fn drop(&mut self) {
        let _ = self.id.compare_exchange(
            self.probe_id,
            0,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        );
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

    /// D-85：取消判据必须按号命中——过期请求不得误伤新探测
    #[test]
    fn cancel_test_only_matches_current_id() {
        assert!(probe_cancel_matches(3, 3), "号一致应命中");
        assert!(!probe_cancel_matches(4, 3), "号已被新探测取代 → 忽略");
        assert!(!probe_cancel_matches(0, 3), "无在途 → 忽略");
        assert!(!probe_cancel_matches(3, 0), "号 0 是哨兵值，永不匹配");
    }

    /// D-85：退出守卫按号条件复位——旧探测退出不得抹掉新探测的号
    #[test]
    fn probe_exit_guard_keeps_newer_id() {
        let id = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(7));
        {
            let _g = ProbeExitGuard {
                id: id.clone(),
                probe_id: 5, // 自己是被取代的那个
            };
        }
        assert_eq!(
            id.load(std::sync::atomic::Ordering::SeqCst),
            7,
            "旧探测退出不得抹掉新探测的在途号"
        );
    }

    /// D-85：退出守卫在"仍是自己"时确实归零（无在途态可恢复）
    #[test]
    fn probe_exit_guard_clears_own_id() {
        let id = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(7));
        {
            let _g = ProbeExitGuard {
                id: id.clone(),
                probe_id: 7,
            };
        }
        assert_eq!(id.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// D-85 §4.7 取代语义（2026-09-11 评审补齐 §6.1 缺失测试）：在途时再启动新
    /// 探测——**置位旧取消标志**（不是拒绝）→ 换新标志 → 换号；被取代的旧探测
    /// 退出时由 `ProbeExitGuard` 条件复位保住新号。
    #[test]
    fn second_test_supersedes_in_flight() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::Arc;
        let in_flight = Arc::new(AtomicU64::new(0));
        let mut cancel = Arc::new(AtomicBool::new(false));
        // 第一次开始（无在途）：不动任何标志
        let c1 = begin_probe_slot(&in_flight, &mut cancel, 1);
        assert!(!c1.load(Ordering::SeqCst), "新探测的取消标志应是干净的");
        assert_eq!(in_flight.load(Ordering::SeqCst), 1);
        // 在途时再启动：旧标志置位、换新标志、号前移
        let c2 = begin_probe_slot(&in_flight, &mut cancel, 2);
        assert!(
            c1.load(Ordering::SeqCst),
            "旧探测必须收到取消（取代而非拒绝）"
        );
        assert!(!c2.load(Ordering::SeqCst), "新探测的取消标志应是干净的");
        assert_eq!(in_flight.load(Ordering::SeqCst), 2, "在途号应换新");
        // 旧探测线程退出（按号条件复位）：不得抹掉新探测的在途号
        {
            let _g = ProbeExitGuard {
                id: in_flight.clone(),
                probe_id: 1,
            };
        }
        assert_eq!(
            in_flight.load(Ordering::SeqCst),
            2,
            "旧探测退出后新探测的在途号必须保留"
        );
        // 新探测正常退出：号归零（恢复"无在途"可再测）
        {
            let _g = ProbeExitGuard {
                id: in_flight.clone(),
                probe_id: 2,
            };
        }
        assert_eq!(in_flight.load(Ordering::SeqCst), 0);
    }
}
