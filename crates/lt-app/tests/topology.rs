//! 依赖拓扑守护（ADR-21：由 scripts/check_deps.ps1 迁入，白名单真源 = 本文件，
//! AGENTS §3 表为文档镜像）。
//!
//! 断言：`crates/*/Cargo.toml` 的内部依赖边（lt-*）必须 ⊆ 下方白名单；
//! `dev-dependencies` 段允许特批追加；normal 段覆盖 dependencies /
//! build-dependencies / target.*.dependencies 三处（旧 PS 版解析不了 target
//! 节与 build-dependencies，此处一并收口）。

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// normal 段白名单（键 = crate 名，值 = 允许的内部依赖）
const NORMAL: &[(&str, &[&str])] = &[
    ("lt-proto", &[]),
    ("lt-i18n", &[]),
    ("lt-models", &["lt-proto"]),
    ("lt-download", &["lt-proto"]),
    ("lt-audio", &["lt-models"]),
    ("lt-asr", &["lt-proto"]),
    ("lt-translate", &["lt-proto"]),
    (
        "lt-orchestrator",
        &[
            "lt-proto",
            "lt-models",
            "lt-download",
            "lt-audio",
            "lt-asr",
            "lt-translate",
        ],
    ),
    ("lt-ui", &["lt-proto", "lt-i18n", "lt-models"]),
    (
        "lt-app",
        &[
            "lt-proto",
            "lt-i18n",
            "lt-models",
            "lt-download",
            "lt-audio",
            "lt-asr",
            "lt-translate",
            "lt-orchestrator",
            "lt-ui",
        ],
    ),
];

/// dev-dependencies 特批（在白名单之上追加；normal 段不适用）
const DEV_EXTRA: &[(&str, &[&str])] = &[
    ("lt-asr", &["lt-models"]),      // 探针共享 probe_models_root（PH-1）
    ("lt-download", &["lt-models"]), // 下载集成测试（真实缓存路径语义）
];

fn internal_deps(table: &toml::Table) -> BTreeSet<String> {
    table
        .keys()
        .filter(|k| k.starts_with("lt-"))
        .cloned()
        .collect()
}

fn normal_deps(doc: &toml::Table) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for key in ["dependencies", "build-dependencies"] {
        if let Some(t) = doc.get(key).and_then(|v| v.as_table()) {
            out.extend(internal_deps(t));
        }
    }
    if let Some(targets) = doc.get("target").and_then(|v| v.as_table()) {
        for cfg in targets.values().filter_map(|v| v.as_table()) {
            if let Some(t) = cfg.get("dependencies").and_then(|v| v.as_table()) {
                out.extend(internal_deps(t));
            }
        }
    }
    out
}

#[test]
fn deps_stay_within_whitelist() {
    // CARGO_MANIFEST_DIR = <repo>/crates/lt-app → 上一级 = crates/
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("manifest 目录必有父级");
    let mut failures: Vec<String> = Vec::new();
    let mut seen = 0;

    for entry in fs::read_dir(crates_dir).expect("crates/ 目录应存在") {
        let manifest = entry.expect("读目录项").path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let doc: toml::Table =
            toml::from_str(&fs::read_to_string(&manifest).expect("读 Cargo.toml"))
                .expect("Cargo.toml 应为合法 toml");
        let name = doc["package"]["name"]
            .as_str()
            .expect("package.name 应为字符串")
            .to_owned();

        let Some((_, normal_ok)) = NORMAL.iter().find(|(k, _)| *k == name) else {
            failures.push(format!(
                "{name}: 不在白名单表里（新 crate？先在本文件登记）"
            ));
            continue;
        };
        seen += 1;
        let dev_ok: BTreeSet<&str> = normal_ok
            .iter()
            .chain(
                DEV_EXTRA
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| *v)
                    .unwrap_or(&[]),
            )
            .copied()
            .collect();

        for dep in normal_deps(&doc) {
            if !normal_ok.contains(&dep.as_str()) {
                failures.push(format!(
                    "{name}: normal 段依赖 {dep} 越界（允许 {normal_ok:?}）"
                ));
            }
        }
        if let Some(dev) = doc.get("dev-dependencies").and_then(|v| v.as_table()) {
            for dep in internal_deps(dev) {
                if !dev_ok.contains(dep.as_str()) {
                    failures.push(format!("{name}: dev 段依赖 {dep} 越界（允许 {dev_ok:?}）"));
                }
            }
        }
    }

    assert_eq!(
        seen, 10,
        "应扫到 10 个 crate，实扫 {seen}（目录结构变了？）"
    );
    assert!(
        failures.is_empty(),
        "依赖拓扑违规：\n{}",
        failures.join("\n")
    );
}
