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

/// 清除全部 `<|...|>` 标签（未闭合标签保留原样，与 sensevoice 同策略）。
pub(crate) fn strip_special_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("<|") {
        out.push_str(&rest[..start]);
        match rest[start..].find("|>") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_special_tags_semantics() {
        // nano 既有语义 1:1（提升共享，防行为漂移）
        assert_eq!(strip_special_tags("<|zh|>你好世界"), "你好世界");
        assert_eq!(strip_special_tags("hello <|HAPPY|>world<|BGM|>"), "hello world");
        // 未闭合标签：保留原样
        assert_eq!(strip_special_tags("abc<|def"), "abc<|def");
        // 纯标签 → 空
        assert_eq!(strip_special_tags("<|BGM|><|Speech|>"), "");
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
