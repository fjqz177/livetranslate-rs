//! 各引擎实现子模块（M5.1 whisper 起用目录组织；sensevoice 暂留 src/sensevoice.rs）。
//!
//! WP-B：`guess_language` / `strip_special_tags` 自 nano 提升为共享后处理
//! （nano 与 qwen3 同为 LLM 解码系，LID 启发式与标签清理语义一致）。

pub mod nano;
pub mod qwen3;
pub mod whisper;

/// 启发式语种判定（原版 _guess_language 1:1；按码点计数）：
/// 假名>0→ja；谚文>30%→ko；汉字>30%→zh；否则 en；空→auto。
pub(crate) fn guess_language(text: &str) -> String {
    let total = text.chars().count();
    if total == 0 {
        return "auto".into();
    }
    let (mut cjk, mut jp, mut ko) = (0usize, 0usize, 0usize);
    for c in text.chars() {
        if ('\u{4e00}'..='\u{9fff}').contains(&c) {
            cjk += 1;
        }
        if ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{31f0}'..='\u{31ff}').contains(&c) {
            jp += 1;
        }
        if ('\u{ac00}'..='\u{d7af}').contains(&c) {
            ko += 1;
        }
    }
    if jp > 0 {
        "ja".into()
    } else if ko as f32 > total as f32 * 0.3 {
        "ko".into()
    } else if cjk as f32 > total as f32 * 0.3 {
        "zh".into()
    } else {
        "en".into()
    }
}

/// 语言值归一（原版 _normalize_language）：空/auto/Auto → None（模型 auto 检测）
pub(crate) fn normalize_language(language: &str) -> Option<String> {
    match language {
        "" | "auto" | "Auto" => None,
        l => Some(l.to_string()),
    }
}

/// 清除全部 `<|...|>` 标签——语义对齐原版正则 `<\|[^|]+\|>`（AH-10/H21）：
/// 标签内容必须为**不含 `|` 的非空串**。畸形标签（`<|a|b|>`、`<||>`、未闭合）
/// 整体不匹配、原样保留——原实现对 `|>` 的贪心搜索会吞掉标签外的文本
pub(crate) fn strip_special_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("<|") {
        out.push_str(&rest[..start]);
        let body = &rest[start + 2..];
        match body.find("|>") {
            // `[^|]+`：`|>` 前必须有至少 1 个非 `|` 字符
            Some(end) if end >= 1 && !body[..end].contains('|') => {
                rest = &body[end + 2..];
            }
            // 该位置不构成合法标签：保留 `<|` 并从其后继续扫描
            //（对齐正则的"匹配失败则前移一位"行为）
            _ => {
                out.push_str("<|");
                rest = body;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 装载失败诊断（AH-7/H14）：sherpa `create` 失败时根因只在 C++ 侧 stderr
/// （父进程 drain_stderr 以 debug 回流），错误文案附清单文件实测字节数——
/// 区分「空文件/截断残留」与「文件在位但 ORT 加载失败」，并引导恢复路径
pub(crate) fn describe_model_files(model_dir: &std::path::Path, files: &[&str]) -> String {
    files
        .iter()
        .map(|name| match std::fs::metadata(model_dir.join(name)) {
            Ok(m) => format!("{name}={}", m.len()),
            Err(_) => format!("{name}=缺失"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// sherpa create 失败错误的公共尾注（AH-7/H14 + DEC-4③ 恢复路径指引）
pub(crate) fn sherpa_create_failure_hint() -> &'static str {
    "详情见日志窗 debug（asr_worker stderr）；若怀疑缓存损坏，可在 设置→数据与存储 删除该模型后重新下载"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_special_tags_semantics() {
        // 正常标签：清除
        assert_eq!(strip_special_tags("<|zh|>你好世界"), "你好世界");
        assert_eq!(
            strip_special_tags("hello <|HAPPY|>world<|BGM|>"),
            "hello world"
        );
        // 未闭合标签：保留原样
        assert_eq!(strip_special_tags("abc<|def"), "abc<|def");
        // 纯标签 → 空
        assert_eq!(strip_special_tags("<|BGM|><|Speech|>"), "");
        // AH-10/H21：畸形标签对齐原版正则 <\|[^|]+\|>——整体不匹配原样保留
        assert_eq!(strip_special_tags("<|a|b|>保留"), "<|a|b|>保留");
        assert_eq!(strip_special_tags("<||>保留"), "<||>保留");
        // 合法标签之后接畸形标签：合法的照清、畸形的保留
        assert_eq!(strip_special_tags("<|zh|>x<|a|b|>y"), "x<|a|b|>y");
    }

    #[test]
    fn guess_language_boundaries() {
        assert_eq!(guess_language("テスト"), "ja");
        assert_eq!(guess_language("こんにちは hello"), "ja");
        assert_eq!(guess_language("한국어 한국어 한국어 abc"), "ko");
        assert_eq!(guess_language("你好世界"), "zh");
        assert_eq!(guess_language("abc 你"), "en"); // 4 码点中 1 汉字 = 25% < 30%
        assert_eq!(guess_language("hello world"), "en");
        assert_eq!(guess_language(""), "auto");
    }
}

/// 验收探针共享（docs/path-hygiene.md PH-1）：模型缓存根——
/// `LIVETRANSLATE_CONFIG_DIR` 优先，回落 `~/.config/livetranslate`
/// （与 lt-models::paths::config_dir 同语义；lt-asr 不依赖 lt-models 故本地实现）。
/// 仅 `--ignored` 探针测试使用。
#[cfg(test)]
pub(crate) fn probe_models_root() -> std::path::PathBuf {
    if let Ok(d) = std::env::var("LIVETRANSLATE_CONFIG_DIR") {
        if !d.is_empty() {
            return std::path::PathBuf::from(d).join("models");
        }
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .expect("探针需要 LIVETRANSLATE_CONFIG_DIR 或 USERPROFILE/HOME");
    std::path::PathBuf::from(home)
        .join(".config")
        .join("livetranslate")
        .join("models")
}
