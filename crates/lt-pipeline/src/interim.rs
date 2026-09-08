//! 增量 ASR（interim）纯算法层：分句 / 短句判定 / 回声剥离 / 比例裁剪 / pending 合并。
//!
//! 移植基准：`LiveTranslate/main.py`（逐字对照）
//! - [`split_sentences`]         ← `_get_segmenter`（L1359）+ `_split_sentences`（L1368）
//! - [`is_short_utterance`]      ← `_is_short_utterance`（L1389）
//! - [`strip_committed_overlap`] ← `_strip_committed_overlap`（L1394）
//! - [`trim_samples`]            ← `_do_interim_asr`（L1411）的比例裁剪分支（L1485-1495）
//! - [`pending_merge`]           ← `_do_interim_asr` L1508 / `_process_interim_final` L1608
//!   的短句 pending 前置拼接（`text = pending + text`，中间无分隔符）
//! - [`committed_prefix`]        ← `_do_interim_asr` L1472-1476（`sentences[:-1]` 拼接）
//! - [`InterimState`]            ← `_interim_*` 状态字段（L262 初始化；L556 / L1217 /
//!   L1242 / L1715 四处等值复位块合并为 [`InterimState::reset`]）
//!
//! 本模块只提供"调用方视角"的纯函数与状态容器：不做线程 / 定时 / 队列装配，
//! 不触碰 [`crate::vad::VadProcessor`]（peek_buffer / trim_front / force_flush 由
//! 主线程接线时按原版 `_do_interim_asr` 的调用顺序组合）。
//!
//! # 自写分句器与 pysbd 的已知偏差（原版经 yasbd→pysbd 分句；Rust 无等价库，在案记录）
//! 1. pysbd 按 `lang` 加载语言专属规则（不支持的语言回落 "en"）；本实现为语言无关的
//!    统一规则（CJK 终止符 + 拉丁终止符两套），`lang` 参数仅保留 API 兼容，不改变行为。
//! 2. 拉丁终止符仅在「后随空白 + 下一字符为大写/数字」时切分；pysbd 对小写开头的
//!    后句也会切（如 "hello. how are you"：pysbd 切两句，本实现保持一句）。
//! 3. 引号/括号规则简化：CJK 终止符后紧邻的右引号/右括号被吸收进当前句；拉丁侧仅
//!    吸收终止符连跑（"..." "?!"）后紧邻的右引号/右括号。pysbd 的嵌套引号/括号
//!    配对规则更完整，本实现不处理嵌套。
//! 4. 缩写保护为有限表（任务指定清单 + 月份 + 常见缩写，可扩充）+ 单字母 initial
//!    规则；pysbd 内置数百条缩写与上下文正则，未收录的缩写后跟大写时可能误切。
//! 5. 每段输出前做 trim 并丢弃空白段（原版 pysbd 输出未显式 trim，clean=False 语义
//!    下通常无首尾空白；此步为无害对齐，同时使 committed 比例不含段间空白）。
//! 6. 长度一律按 Unicode 标量字符计数（对齐 Python `len(str)` 的 code point 语义；
//!    与 Rust `str::len()` 的字节语义不同）。

// ─────────────────────────── 分句 ───────────────────────────

/// 缩写保护表（小写；对照原版 pysbd 的缩写规则子集，可扩充）。
/// 任务指定：Mr. Mrs. Dr. Prof. St. vs. etc. e.g. i.e. No. Fig. al. Inc. Ltd. Co.
/// Corp. + 月份缩写 Jan.-Dec.；另扩充常见缩写与 a.m / p.m / U.S / U.K。
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "mt", "ft", //
    "vs", "etc", "e.g", "i.e", "no", "fig", "al", "inc", "ltd", "co", //
    "corp", "dept", "univ", "ave", "apt", "a.m", "p.m", "u.s", "u.k", //
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", //
    "sep", "sept", "oct", "nov", "dec",
];

/// CJK 句终止符（任务指定：。！？…）
fn is_cjk_terminator(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '…')
}

/// 拉丁句终止符（任务指定：.!?）
fn is_latin_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// 右引号 / 右括号（CJK 终止符后允许吸收，归入当前句）
fn is_closer(c: char) -> bool {
    matches!(
        c,
        '」' | '』' | '）' | '】' | '〉' | '》' | '〕' | '"' | '\'' | ')' | ']' | '}'
    )
}

/// 终止符 '.' 之前紧邻的词是否为缩写（对照 pysbd 缩写保护）。
/// 规则：向回收集字母与 '.'，小写折叠后查表；单个字母视为人名 initial 一样保护。
fn is_abbreviation(chars: &[char], dot_idx: usize) -> bool {
    let mut collected: Vec<char> = Vec::new();
    let mut k = dot_idx;
    while k > 0 {
        let c = chars[k - 1];
        if c.is_alphabetic() || c == '.' {
            collected.push(c);
            k -= 1;
        } else {
            break;
        }
    }
    collected.reverse();
    let word: String = collected.iter().collect::<String>().to_lowercase();
    if word.is_empty() {
        return false;
    }
    if word.chars().count() == 1 && word.chars().next().is_some_and(char::is_alphabetic) {
        return true; // 单字母 initial（如 "John Q. Public"）
    }
    ABBREVIATIONS.contains(&word.as_str())
}

/// 分句（原版 `_split_sentences`：yasbd→pysbd 分句 + 长句逗号回退）。
///
/// - CJK 终止符（。！？…）后切分；后续字符为终止/引号/右括号时并入当前句继续；
/// - 拉丁终止符（.!?）后随空白 + 大写/数字时切分，带缩写保护与小数点保护（3.5）；
/// - 逗号回退（原版逐字）：单段长文本无句切时，含「、」阈值 25 字符否则 60，
///   从 len-8 向 6 反查 `,，;；、`，before>15 且 after>3 才切两段（取最右一个）；
/// - 每段 trim，过滤空白段（已知偏差 #5）。
///
/// `_lang` 仅保留原版 API 形参（pysbd 按 lang 选规则；本实现语言无关，见偏差 #1）。
pub fn split_sentences(text: &str, _lang: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut ends: Vec<usize> = Vec::new(); // 切分点：句子 = chars[start..end]

    let mut i = 0;
    while i < n {
        let c = chars[i];
        if is_cjk_terminator(c) {
            // 连续终止符与紧邻右引号/右括号归入当前句，遇其它字符即断开
            let mut j = i + 1;
            while j < n && (is_cjk_terminator(chars[j]) || is_closer(chars[j])) {
                j += 1;
            }
            ends.push(j);
            i = j;
        } else if is_latin_terminator(c) {
            // 小数点保护：digit.digit 不切（如 3.5）
            if c == '.'
                && i > 0
                && chars[i - 1].is_numeric()
                && chars.get(i + 1).is_some_and(|&d| d.is_numeric())
            {
                i += 1;
                continue;
            }
            // 拉丁终止符连跑（"..." "?!"）
            let mut j = i + 1;
            while j < n && is_latin_terminator(chars[j]) {
                j += 1;
            }
            // 缩写保护（仅 '.' 有缩写语义）
            if c == '.' && is_abbreviation(&chars, i) {
                i = j;
                continue;
            }
            // 终止符连跑后紧邻的右引号/右括号归入当前句
            let mut k = j;
            while k < n && is_closer(chars[k]) {
                k += 1;
            }
            // 切分条件：后随空白 + 下一个大写/数字
            let mut m = k;
            while m < n && chars[m].is_whitespace() {
                m += 1;
            }
            if m > k && m < n && (chars[m].is_uppercase() || chars[m].is_numeric()) {
                ends.push(k);
                i = k;
                continue;
            }
            i = j;
        } else {
            i += 1;
        }
    }

    let mut parts: Vec<String> = Vec::new();
    let mut start = 0;
    for &e in &ends {
        let part = chars[start..e].iter().collect::<String>();
        let part = part.trim();
        if !part.is_empty() {
            parts.push(part.to_string());
        }
        start = e;
    }
    let tail = chars[start..].iter().collect::<String>();
    let tail = tail.trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }

    // 原版：多于一段直接返回
    if parts.len() > 1 {
        return parts;
    }

    // 逗号回退（原版逐字）：长句无句读时在最后一个平衡逗号处切两段
    // CJK「、」阈值 25 字符；其余逗号族阈值 60（长句降低延迟）
    let min_len = if chars.contains(&'、') { 25 } else { 60 };
    let total = chars.len();
    if total > min_len {
        // 原版 range(len-8, 5, -1)：i 从 len-8 反查到 6
        for i in (6..=total.saturating_sub(8)).rev() {
            if matches!(chars[i], ',' | '，' | ';' | '；' | '、') {
                let before = chars[..=i].iter().collect::<String>();
                let before = before.trim();
                let after = chars[i + 1..].iter().collect::<String>();
                let after = after.trim();
                if !before.is_empty()
                    && !after.is_empty()
                    && before.chars().count() > 15
                    && after.chars().count() > 3
                {
                    return vec![before.to_string(), after.to_string()];
                }
            }
        }
    }

    parts
}

// ─────────────────────────── 短句判定 ───────────────────────────

/// 短句判定（原版 `_is_short_utterance` 逐字）：字母数字字符 ≤8 视为
/// 噪声/语气词/碎片，缓冲进 pending 等下句合并。
/// Python `str.isalnum` ↔ Rust `char::is_alphanumeric`（CJK/假名/谚文均计入）。
pub fn is_short_utterance(text: &str) -> bool {
    let alnum = text.chars().filter(|c| c.is_alphanumeric()).count();
    alnum <= 8
}

// ─────────────────────────── 回声剥离 ───────────────────────────

/// 剥离与上次已提交内容重叠的回声（原版 `_strip_committed_overlap` 逐字）。
///
/// `committed_tail` 为上次提交文本的末尾（调用方负责截取 ≤50 字符，原版 L1523）。
/// 匹配：tail 小写折叠 + 去尾空白；text 小写折叠后取前缀与 tail 后缀比较，
/// overlap 长度从 min(len(tail), len(text)) 降到 3（原版 `range(max_check, 2, -1)`）。
/// 命中后剥前 overlap 个字符并 trim：非空则返回；全剥空返回空串；未命中原样返回。
pub fn strip_committed_overlap(text: &str, committed_tail: &str) -> String {
    if committed_tail.is_empty() {
        return text.to_string();
    }
    // 原版：tail = committed_tail.lower().rstrip()；text_lower = text.lower()
    let tail: Vec<char> = committed_tail.to_lowercase().trim_end().chars().collect();
    let text_lower: Vec<char> = text.to_lowercase().chars().collect();
    let max_check = tail.len().min(text_lower.len());
    for overlap_len in (3..=max_check).rev() {
        if text_lower[..overlap_len] == tail[tail.len() - overlap_len..] {
            // 剥离后非空才剥；全剥空返回空串（原版两分支合一）
            // 注：按 char 数对齐原 text（lowercase 极端改变字符数的病态输入除外）
            let stripped: String = text.chars().skip(overlap_len).collect();
            return stripped.trim().to_string();
        }
    }
    text.to_string()
}

// ─────────────────────────── 提交前缀 ───────────────────────────

/// 已完结句拼接（原版 `_do_interim_asr` L1471-1476）：除最后一句外原样拼接
/// （各句自带句末标点，无分隔符）；不足两句返回空串（原版 `sentences[:-1]` 语义）。
pub fn committed_prefix(sentences: &[String]) -> String {
    if sentences.len() <= 1 {
        return String::new();
    }
    sentences[..sentences.len() - 1].concat()
}

// ─────────────────────────── 比例裁剪 ───────────────────────────

/// 按字符比例计算已消费音频的裁剪样本数（原版 `_do_interim_asr` L1485-1495 逐字）。
///
/// - 比例 = committed_chars / full_chars × total + 0.3s 余量（margin，防重复识别）；
/// - 上限：为剩余句保留至少 0.5s（`max_trim`，总样本不足 0.5s 时为 0）；
/// - 下限：trim < 0.3s 且 >0 时取 min(0.3s, total/2)（防重复识别循环）；
/// - 守卫：full_chars == 0 → 0（原版 `max(len(full_text), 1)` 防除零，但调用点已保证
///   full_text 非空，等价改写）；committed_chars == 0 → 0（原版调用前
///   `if not committed_text.strip(): return False`，未提交不裁剪）。
pub fn trim_samples(
    total_samples: usize,
    committed_chars: usize,
    full_chars: usize,
    sample_rate: usize,
) -> usize {
    if full_chars == 0 || committed_chars == 0 {
        return 0;
    }
    let sr = sample_rate as f64;
    // 原版：ratio = len(committed_text) / max(len(full_text), 1)
    let ratio = committed_chars as f64 / full_chars as f64;
    let margin = (0.3 * sr) as usize; // 原版 int(0.3 * 16000)
    let mut trim = (ratio * total_samples as f64) as usize + margin;
    // 不过度裁剪：为剩余句保留至少 0.5s（原版 max_trim = total - int(0.5*16000)）
    let max_trim = total_samples.saturating_sub((0.5 * sr) as usize);
    trim = trim.min(max_trim);
    // 最小裁剪防重复识别循环，且不超过总样本一半（原版 L1492-1495）
    let min_trim = (0.3 * sr) as usize;
    if trim < min_trim && trim > 0 {
        trim = min_trim.min(total_samples / 2);
    }
    trim
}

// ─────────────────────────── pending 合并 ───────────────────────────

/// 短句 pending 前置合并（原版 `_do_interim_asr` L1508-1510 /
/// `_process_interim_final` L1608-1610 逐字）：pending 非空则
/// `text = pending + text`，中间**无分隔符**（短句自带标点）。
/// 调用方传入各自已 strip 的文本；合并后调用方清空 pending。
pub fn pending_merge(pending: &str, text: &str) -> String {
    if pending.is_empty() {
        text.to_string()
    } else {
        format!("{pending}{text}")
    }
}

// ─────────────────────────── 状态容器 ───────────────────────────

/// 增量 ASR 状态（原版 `_interim_*` 字段；不含线程/定时，接线时由调用方驱动）。
#[derive(Debug, Clone, Default)]
pub struct InterimState {
    /// 上次已提交文本的末尾（原版 `_interim_committed_tail`；提交后由调用方更新为
    /// committed_text 的末 ≤50 字符，原版 L1523），用于 [`strip_committed_overlap`] 回声剥离
    pub committed_tail: String,
    /// 短句缓冲（原版 `_interim_pending`），[`pending_merge`] 后清空
    pub pending: String,
    /// 上次 interim 检查时的 VAD 缓冲样本数（原版 `_last_interim_samples`）
    pub last_samples: usize,
    /// 上次 interim 检查的时间戳（原版 `_last_interim_check_time`，perf_counter 秒）
    pub last_check_time: f64,
    /// 本轮语音内已有增量提交（原版 `_interim_active`；vad_flush 到来时为真则
    /// 收尾段走回声剥离 + pending 拼接语义，随后随 [`Self::reset`] 复位）
    pub active: bool,
}

impl InterimState {
    /// 全量复位（原版 L556 / L1217 / L1242 / L1715 四处五连复位的等值合并）
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

// ─────────────────────────── 测试 ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── 分句：缩写与小数点保护（对照 test_segmentation.py）──

    #[test]
    fn split_english_abbreviation_protection() {
        // 对照 test_segmentation.py::test_adapter_handles_english_abbreviations
        let parts = split_sentences("Mr. Smith paid 3.5 dollars. Dr. Lee disagreed.", "en");
        assert_eq!(
            parts,
            vec!["Mr. Smith paid 3.5 dollars.", "Dr. Lee disagreed."]
        );
    }

    #[test]
    fn split_month_abbreviation_protection() {
        // "Jan. 2020" 的 . 后跟数字，若不在表内会被误切
        let parts = split_sentences("In Jan. 2020 we met. She left.", "en");
        assert_eq!(parts, vec!["In Jan. 2020 we met.", "She left."]);
    }

    #[test]
    fn split_decimal_not_split() {
        let parts = split_sentences("It costs 3.5 dollars in total.", "en");
        assert_eq!(parts, vec!["It costs 3.5 dollars in total."]);
    }

    #[test]
    fn split_single_letter_initial_protected() {
        let parts = split_sentences("John Q. Public spoke. Then he left.", "en");
        assert_eq!(parts, vec!["John Q. Public spoke.", "Then he left."]);
    }

    #[test]
    fn split_lowercase_after_period_not_split() {
        // 已知偏差 #2：pysbd 会切，本实现按任务规则不切
        let parts = split_sentences("hello. how are you", "en");
        assert_eq!(parts, vec!["hello. how are you"]);
    }

    #[test]
    fn split_latin_terminator_run_and_capital() {
        // "?!" 连跑 + 空白 + 大写 → 切
        let parts = split_sentences("What?! Next part", "en");
        assert_eq!(parts, vec!["What?!", "Next part"]);
    }

    // ── 分句：CJK ──

    #[test]
    fn split_japanese_basic() {
        // 对照 test_segmentation.py::test_adapter_keeps_pysbd_interface
        let parts = split_sentences("今日はいい天気ですね。散歩に行きましょう。", "ja");
        assert_eq!(
            parts,
            vec!["今日はいい天気ですね。", "散歩に行きましょう。"]
        );
    }

    #[test]
    fn split_chinese_basic() {
        let parts = split_sentences("今天天气很好。我们去公园散步。", "zh");
        assert_eq!(parts, vec!["今天天气很好。", "我们去公园散步。"]);
    }

    #[test]
    fn split_cjk_question_exclamation() {
        let parts = split_sentences("真的吗？太棒了！", "zh");
        assert_eq!(parts, vec!["真的吗？", "太棒了！"]);
    }

    #[test]
    fn split_cjk_ellipsis_run() {
        // "……" 连续终止符并入一次切分
        let parts = split_sentences("等等……然后呢？", "zh");
        assert_eq!(parts, vec!["等等……", "然后呢？"]);
    }

    #[test]
    fn split_cjk_closing_quote_absorbed() {
        // 。 后的 」 归入当前句（任务规则 a）
        let parts = split_sentences("他说：「你好。」然后离开。", "zh");
        assert_eq!(parts, vec!["他说：「你好。」", "然后离开。"]);
    }

    // ── 分句：逗号回退（原版逐字阈值与反查）──

    #[test]
    fn comma_fallback_over_60_chars() {
        // 67 字符、无「、」、无句读 → 阈值 60，在唯一 ',' 处切两段
        let text = "这是一个特别长的句子没有任何句读符号,后面还有更多内容需要继续说完才能构成一段完整的话至少要有一些字数才好这样才满足切分条件的长度要求";
        assert_eq!(text.chars().count(), 67);
        let parts = split_sentences(text, "zh");
        assert_eq!(
            parts,
            vec![
                "这是一个特别长的句子没有任何句读符号,",
                "后面还有更多内容需要继续说完才能构成一段完整的话至少要有一些字数才好这样才满足切分条件的长度要求",
            ]
        );
    }

    #[test]
    fn comma_fallback_touten_over_25_chars() {
        // 含「、」→ 阈值 25；47 字符在「、」处切（before 26>15，after 21>3）
        let text = "这份清单里面包括了很多种不同的水果和蔬菜比如说苹果、香蕉还有橙子和葡萄等等另外还有一些绿叶蔬菜";
        assert_eq!(text.chars().count(), 47);
        let parts = split_sentences(text, "zh");
        assert_eq!(
            parts,
            vec![
                "这份清单里面包括了很多种不同的水果和蔬菜比如说苹果、",
                "香蕉还有橙子和葡萄等等另外还有一些绿叶蔬菜",
            ]
        );
    }

    #[test]
    fn comma_fallback_picks_rightmost_balanced() {
        // 72 字符两个逗号都可用 → 反查（从 len-8 向 6）取最右一个切分
        let text = "前半段内容足够长包含了足够的汉字字符数量,中间部分也很长包含了不少内容用于测试反查逻辑的走向,末尾部分同样需要具备一定长度才能让整段文本超过阈值";
        assert_eq!(text.chars().count(), 72);
        let parts = split_sentences(text, "zh");
        assert_eq!(
            parts,
            vec![
                "前半段内容足够长包含了足够的汉字字符数量,中间部分也很长包含了不少内容用于测试反查逻辑的走向,",
                "末尾部分同样需要具备一定长度才能让整段文本超过阈值",
            ]
        );
    }

    #[test]
    fn comma_fallback_not_triggered_on_short_text() {
        // 10 字符 ≤ 60 且无「、」→ 不回退，整段为一句
        let parts = split_sentences("你好啊,今天天气不错", "zh");
        assert_eq!(parts, vec!["你好啊,今天天气不错"]);
    }

    // ── 回声剥离 ──

    #[test]
    fn overlap_tail_hit() {
        // text 前缀 == tail 后缀（5 字符 "world"）→ 剥前 5 字符
        let out = strip_committed_overlap("world and more words", "hello world");
        assert_eq!(out, "and more words");
    }

    #[test]
    fn overlap_partial_hit() {
        // text 前缀 "fox" == tail 末 3 字符（部分重叠）→ 剥 3 字符
        let out = strip_committed_overlap("fox jumps high", "the quick brown fox");
        assert_eq!(out, "jumps high");
    }

    #[test]
    fn overlap_no_hit_returns_text() {
        let out = strip_committed_overlap("xyz goes on", "abc def");
        assert_eq!(out, "xyz goes on");
    }

    #[test]
    fn overlap_full_strip_returns_empty() {
        // 整段都是回声：剥完为空 → 返回空串（原版 return "" 分支）
        let out = strip_committed_overlap("world", "hello world");
        assert_eq!(out, "");
    }

    #[test]
    fn overlap_case_folding_and_tail_rstrip() {
        // tail 大写折叠 + 尾部空白被 rstrip（原版 .lower().rstrip()）
        let out = strip_committed_overlap("WORLD and more", "hello WORLD   ");
        assert_eq!(out, "and more");
    }

    #[test]
    fn overlap_empty_tail_returns_text() {
        let out = strip_committed_overlap("fresh text", "");
        assert_eq!(out, "fresh text");
    }

    // ── 比例裁剪（sample_rate 默认按 16000 语义断言）──

    #[test]
    fn trim_proportional_with_margin() {
        // ratio=0.5×160000=80000 + 0.3s(4800) = 84800；上限 152000 不约束
        assert_eq!(trim_samples(160_000, 500, 1000, 16_000), 84_800);
    }

    #[test]
    fn trim_full_committed_caps_at_half_second() {
        // full==committed → ratio=1 → 超上限，截到 total-0.5s
        assert_eq!(trim_samples(160_000, 200, 200, 16_000), 152_000);
    }

    #[test]
    fn trim_zero_committed_returns_zero() {
        assert_eq!(trim_samples(160_000, 0, 1000, 16_000), 0);
    }

    #[test]
    fn trim_zero_full_returns_zero() {
        assert_eq!(trim_samples(160_000, 500, 0, 16_000), 0);
    }

    #[test]
    fn trim_small_total_caps_at_total_half() {
        // total=1s：max_trim = 16000-8000 = 8000 = total/2
        assert_eq!(trim_samples(16_000, 50, 50, 16_000), 8_000);
    }

    #[test]
    fn trim_min_floor_overrides_cap_with_half_limit() {
        // total=0.6s：max_trim=1600 < 0.3s(4800) 且 >0 → 取 min(4800, total/2)=4800
        // （原版 L1492-1495 的下限分支会覆盖 0.5s 上限，1:1 保留）
        assert_eq!(trim_samples(9_600, 10, 10, 16_000), 4_800);
    }

    #[test]
    fn trim_tiny_total_returns_zero() {
        // total < 0.5s：max_trim 饱和为 0 → trim=0（原版 max(max_trim, 0)）
        assert_eq!(trim_samples(4_000, 10, 10, 16_000), 0);
    }

    #[test]
    fn trim_scales_with_sample_rate() {
        // 48k：ratio 0.5×48000=24000 + 14400 = 38400 → 上限 48000-24000=24000
        assert_eq!(trim_samples(48_000, 500, 1000, 48_000), 24_000);
    }

    // ── 短句判定 ──

    #[test]
    fn short_utterance_english() {
        assert!(is_short_utterance("okay")); // 4 字母
        assert!(is_short_utterance("abcdefgh")); // 边界：恰 8
        assert!(!is_short_utterance("abcdefghi")); // 9 → 非短句
        assert!(!is_short_utterance("hello world again"));
    }

    #[test]
    fn short_utterance_cjk_counts_alnum() {
        assert!(is_short_utterance("好的")); // 2 汉字
        assert!(is_short_utterance("好，谢谢。")); // 标点不计，4 字
        assert!(is_short_utterance("こんにちは")); // 假名计入 isalnum
        assert!(!is_short_utterance("今天天气真的非常好了")); // 10 字 → 非短句
    }

    #[test]
    fn short_utterance_boundary_at_eight() {
        assert!(is_short_utterance("今天天气非常好啊")); // 8 → true（≤8）
        assert!(!is_short_utterance("今天天气非常好了呀")); // 9 → false
        assert!(!is_short_utterance("123456789")); // 9 数字 → false
    }

    // ── pending 合并 ──

    #[test]
    fn pending_merge_prepends_without_separator() {
        // 原版 L1508-1510：text = pending + text，中间无分隔符
        assert_eq!(
            pending_merge("好的。", "Hello world."),
            "好的。Hello world."
        );
    }

    #[test]
    fn pending_merge_empty_pending_is_identity() {
        assert_eq!(pending_merge("", "Hello world."), "Hello world.");
    }

    // ── 提交前缀 ──

    #[test]
    fn committed_prefix_joins_all_but_last() {
        let sentences = vec!["A.".to_string(), "B.".to_string(), "C.".to_string()];
        assert_eq!(committed_prefix(&sentences), "A.B.");
    }

    #[test]
    fn committed_prefix_single_or_empty_is_blank() {
        assert_eq!(committed_prefix(&["Only.".to_string()]), "");
        assert_eq!(committed_prefix(&[]), "");
    }

    // ── 状态容器 ──

    #[test]
    fn interim_state_reset_restores_defaults() {
        let mut st = InterimState {
            committed_tail: "previous tail".into(),
            pending: "好的。".into(),
            last_samples: 32_000,
            last_check_time: 123.5,
            active: true,
        };
        st.reset();
        assert_eq!(st.committed_tail, "");
        assert_eq!(st.pending, "");
        assert_eq!(st.last_samples, 0);
        assert_eq!(st.last_check_time, 0.0);
        assert!(!st.active);
        // 默认值与 reset 后一致
        assert_eq!(st.committed_tail, InterimState::default().committed_tail);
    }
}
