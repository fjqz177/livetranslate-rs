//! 仓库文本卫生守护（ADR-21：由 scripts/check_personal_paths.ps1 迁入）。
//!
//! - ① 无 BOM（头三字节 EF BB BF 即红，ADR-20）
//! - ② 严格 UTF-8（非法字节即红）
//! - （index LF 不查——.gitattributes 已全仓锁 LF，ADR-20；双真源必漂移）
//! - Tier1 = 本机用户名 / 实名用户目录路径：任何文件命中即红（`<占位符>` 豁免；
//!   本头注刻意不写路径示例字面量——写了会被本守护自己的 Tier1 命中）。
//!   唯一豁免形态 = 用户名紧跟 `/sherpa-onnx-` 的仓 ID（D-132：registry 登记
//!   的 MS 镜像下载源，刻意公开非泄漏；目录路径形态仍由 Users 路径规则拦截）
//! - Tier2 = 剥除 URL / 系统级白名单（Windows、Program Files）/ 占位符 Users 路径后，
//!   其余盘符路径：archive 外红、archive 内仅提示（史档快照降档）
//!
//! 与原脚本的差异（均为有意修复）：读取失败的文件显式红（原版静默跳过是最大盲区）；
//! Tier1 分隔符 `[/\]{1,2}` 对齐 Tier2（原版单分隔符漏双反斜杠转义写法）。

use std::fs;
use std::path::Path;
use std::process::Command;

const BINARY_EXT: &[&str] = &[
    "png", "jpg", "ico", "icns", "onnx", "dll", "br", "bin", "ttf", "otf", "woff", "woff2", "zip",
    "gz",
];

fn repo_root() -> &'static Path {
    // CARGO_MANIFEST_DIR = <repo>/crates/lt-app → 两级上溯 = 仓库根
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("仓库根定位失败")
}

fn is_text_file(rel: &str) -> bool {
    match rel.rsplit_once('.') {
        Some((_, ext)) => !BINARY_EXT.contains(&ext),
        None => true,
    }
}

/// Tier1 用户名轴判定器（D-132 仓 ID 形态豁免；构造一次、逐行复用）：
/// `<用户名>/sherpa-onnx-…` 仓 ID 形态（registry 登记的 MS 镜像下载源，刻意公开
/// 非泄漏）豁免——同行兼含仓 ID 与其余裸用户名泄漏时仍命中（段级剥除后评估）；
/// 目录路径形态不含仓 ID 前缀、不受此豁免影响（Users 路径由扫描处 users_re
/// 独立拦截）。用户名从环境现取传入，本文件不落字面量（写了会被自家 Tier1 命中）。
struct Tier1UserChecker {
    user_re: regex::Regex,
    ms_repo_re: regex::Regex,
}

impl Tier1UserChecker {
    fn new(user: &str) -> Option<Self> {
        if user.is_empty() {
            return None;
        }
        Some(Self {
            user_re: regex::Regex::new(&regex::escape(user)).expect("用户名转义后必为合法正则"),
            ms_repo_re: regex::Regex::new(&format!(r"{}[/]sherpa-onnx-", regex::escape(user)))
                .expect("MS 仓 ID 正则必合法"),
        })
    }

    fn hit(&self, line: &str) -> bool {
        // 段级剥除：先移除全部仓 ID 匹配段，再查裸用户名——同行兼含仓 ID 与
        // 其余裸用户名泄漏时仍命中（replace_all 防双仓 ID 行留残余误报）
        self.user_re
            .is_match(&self.ms_repo_re.replace_all(line, ""))
    }
}

#[test]
fn tracked_text_files_are_clean() {
    // git ls-files 从仓库根跑；quotepath=false 防非 ASCII 路径被引号转义
    let out = Command::new("git")
        .args(["-c", "core.quotepath=false", "ls-files"])
        .current_dir(repo_root())
        .output()
        .expect("git ls-files 应可执行（须在源码仓内跑）");
    assert!(out.status.success(), "git ls-files 退出非 0：{out:?}");
    let listing = String::from_utf8(out.stdout).expect("ls-files 输出应为 UTF-8");

    let user = std::env::var("USERNAME").unwrap_or_default();
    // Tier1 用户名轴判定器（D-132 仓 ID 形态豁免内置，见类型 doc）
    let tier1_user_check = Tier1UserChecker::new(&user);
    // Tier1 实名 Users 路径（不区分大小写；< 占位符豁免由匹配后字符判定实现——
    // rust regex 无 lookaround，等价于原版 (?!<)）
    let users_re =
        regex::Regex::new(r"(?i)C:[/\\]{1,2}Users[/\\]{1,2}").expect("Tier1 正则常量合法");
    // Tier2 剥除链（与原版同序）：URL → 系统级白名单 → 占位符 Users 路径
    let scrub_url = regex::Regex::new(r"(?i)\b(?:https?|ftp)://\S+").unwrap();
    let scrub_sys = regex::Regex::new(r"(?i)\bC:[/\\]{1,2}(?:Windows|Program Files)\b\S*").unwrap();
    let scrub_ph = regex::Regex::new(r"(?i)\bC:[/\\]{1,2}Users[/\\]{1,2}<[^>]*>").unwrap();
    let drive_re = regex::Regex::new(r"\b[A-Za-z]:[/\\]").expect("Tier2 正则常量合法");

    let mut failures: Vec<String> = Vec::new();
    let mut archive_warns = 0;

    for rel in listing.lines().filter(|l| !l.is_empty()) {
        if !is_text_file(rel) {
            continue;
        }
        let bytes = match fs::read(repo_root().join(rel)) {
            Ok(b) => b,
            Err(e) => {
                // 原版静默跳过（catch { continue }）——其失败方向是"开放"，这里显式红
                failures.push(format!("{rel}: 读取失败（{e}）——未检查，须人工确认"));
                continue;
            }
        };
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            failures.push(format!(
                "{rel}: UTF-8 BOM（头三字节 EF BB BF）——全仓无 BOM（ADR-20）"
            ));
        }
        if std::str::from_utf8(&bytes).is_err() {
            failures.push(format!("{rel}: 非法 UTF-8（疑非 UTF-8 编码入库）"));
        }

        let text = match std::str::from_utf8(&bytes) {
            Ok(t) => t,
            Err(_) => continue, // 编码已红，行级扫描无意义
        };
        let in_archive = rel.starts_with("docs/archive/");
        for (i, line) in text.lines().enumerate() {
            let n = i + 1;
            // D-132 豁免在 Tier1UserChecker 内评估（仓 ID 形态；同行其余裸
            // 用户名泄漏与目录路径形态仍命中）
            let t1_user = tier1_user_check.as_ref().is_some_and(|c| c.hit(line));
            let t1_users = users_re.find(line).is_some_and(|m| {
                // 匹配段以 '<' 开头 → 占位符路径（<…>），豁免
                !line[m.end()..].starts_with('<')
            });
            if t1_user || t1_users {
                failures.push(format!("{rel}:{n}: {}", line.trim()));
                continue;
            }
            let scrubbed = scrub_url.replace_all(line, "");
            let scrubbed = scrub_sys.replace_all(&scrubbed, "");
            let scrubbed = scrub_ph.replace_all(&scrubbed, "");
            if drive_re.is_match(&scrubbed) {
                if in_archive {
                    archive_warns += 1; // 史档快照降档：提示不阻断（eprintln 随测试输出可见）
                } else {
                    failures.push(format!("{rel}:{n}: {}", line.trim()));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "个人路径 / 文本卫生命中 {} 处：\n{}\n（处置见 docs/archive/path-hygiene.md 与 text-hygiene.md）",
        failures.len(),
        failures.join("\n")
    );
    if archive_warns > 0 {
        eprintln!("note: archive 内 Tier2 提示 {archive_warns} 处（不阻断）");
    }
}

#[test]
fn tier1_waives_ms_repo_id_form_only() {
    // D-132：registry 的 MS 镜像仓 ID 形如「用户名/sherpa-onnx-…」（刻意公开的
    // 下载源）→ 豁免；但豁免必须最小开口——同行其余裸用户名泄漏与目录路径
    // 形态不得被掩盖。用户名从环境现取——本文件不落字面量（写了会被自家
    // Tier1 命中）。
    let user = std::env::var("USERNAME").unwrap_or_default();
    let Some(check) = Tier1UserChecker::new(&user) else {
        return; // 无用户名环境无 Tier1 用户名面，无可验对象
    };
    // 仓 ID 形态 → 豁免
    assert!(
        !check.hit(&format!("{user}/sherpa-onnx-funasr-nano-int8-2025-12-30")),
        "仓 ID 形态应豁免"
    );
    // 同行双仓 ID → 均豁免（不得因剥除方式留残余裸用户名而误报）
    assert!(
        !check.hit(&format!("{user}/sherpa-onnx-a 与 {user}/sherpa-onnx-b")),
        "同行双仓 ID 均应剥除豁免"
    );
    // 混合行：仓 ID + 同行裸用户名泄漏 → 仍命中（豁免不得掩盖行内其余泄漏）
    assert!(
        check.hit(&format!("见 {user}/sherpa-onnx-x，联系 {user}@example.com")),
        "混合行的裸用户名泄漏不得被仓 ID 豁免掩盖"
    );
    // 目录路径形态 → 仍命中（不含仓 ID 前缀，豁免不波及；Users 路径另由
    // users_re 独立拦截）。路径字面量运行时拼接——源码行不得出现
    // 「C: + 分隔符 + Users」字面量，否则被本守护自己的 Tier1 扫中（头注纪律）
    let sep = std::path::MAIN_SEPARATOR.to_string();
    assert!(
        check.hit(&format!("C:{sep}Users{sep}{user}{sep}livetranslate")),
        "目录路径形态不得被仓 ID 豁免波及"
    );
}
