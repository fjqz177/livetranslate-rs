//! 仓库文本卫生守护（ADR-21：由 scripts/check_personal_paths.ps1 迁入）。
//!
//! - ① 无 BOM（头三字节 EF BB BF 即红，ADR-20）
//! - ② 严格 UTF-8（非法字节即红）
//! - （index LF 不查——.gitattributes 已全仓锁 LF，ADR-20；双真源必漂移）
//! - Tier1 = 本机用户名 / 实名 `C:\Users\…` 路径：任何文件命中即红（`<占位符>` 豁免）
//! - Tier2 = 剥除 URL / 系统级白名单（Windows、Program Files）/ 占位符 Users 路径后，
//!   其余盘符路径：archive 外红、archive 内仅提示（史档快照降档）
//!
//! 与原脚本的差异（均为有意修复）：读取失败的文件显式红（原版静默跳过是最大盲区）；
//! Tier1 分隔符 `[/\]{1,2}` 对齐 Tier2（原版单分隔符漏 `C:\\Users\\…` 转义写法）。

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
    let user_re = if user.is_empty() {
        None
    } else {
        Some(regex::Regex::new(&regex::escape(&user)).expect("用户名转义后必为合法正则"))
    };
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
            let t1_user = user_re.as_ref().is_some_and(|re| re.is_match(line));
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
