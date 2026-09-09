//! 设置偏离默认值检测（serde JSON 递归 diff；N4）。
//!
//! 思路：`Settings` 已 derive PartialEq，但「逐字段路径」提示需要字段级定位——
//! 对 `Settings::default()` 与当前设置做一次 serde_json 递归对比，产出偏离路径
//! 集合（如 `style.bg_color`、`subtitle_mode.sentences`、`models[0].api_base`）。
//! 后续新增字段自动覆盖（diff 基于序列化，不感知字段），测试集中一处。
//!
//! 渲染消费：组标题 ●（该组含偏离字段）+ 页顶「恢复本页默认值」按钮的显示条件。

use serde_json::Value;
use std::sync::OnceLock;

/// 与 Settings::default() 比对的偏离路径集合（空 = 全默认）
pub fn diff_paths(settings: &lt_proto::Settings) -> Vec<String> {
    static DEFAULTS: OnceLock<Value> = OnceLock::new();
    let def = DEFAULTS.get_or_init(|| {
        serde_json::to_value(lt_proto::Settings::default()).expect("Settings 必可序列化")
    });
    let cur = serde_json::to_value(settings).expect("Settings 必可序列化");
    diff_settings(def, &cur)
}

/// 两 Value 递归 diff：返回偏离路径（a = 基准/默认，b = 当前）
pub fn diff_settings(a: &Value, b: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk("", a, b, &mut out);
    out
}

fn child(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn walk(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(ao), Value::Object(bo)) => {
            // 以默认侧键域为基准；当前侧多余键也计入（理论不可达，防御性）
            for (k, av) in ao {
                match bo.get(k) {
                    Some(bv) => walk(&child(path, k), av, bv, out),
                    None => out.push(child(path, k)),
                }
            }
            for k in bo.keys() {
                if !ao.contains_key(k) {
                    out.push(child(path, k));
                }
            }
        }
        (Value::Array(aa), Value::Array(ba)) => {
            if aa.len() != ba.len() {
                // 数组长度不同：整条记录（如 models 集合变化、subtitle 行增减）
                out.push(path.to_string());
            } else {
                for (i, (av, bv)) in aa.iter().zip(ba.iter()).enumerate() {
                    let p = format!("{path}[{i}]");
                    walk(&p, av, bv, out);
                }
            }
        }
        _ => {
            if a != b {
                out.push(path.to_string());
            }
        }
    }
}

/// 路径集合是否命中任一前缀（组级判定：组内字段路径以给定前缀开头）
pub fn any_dirty(diffs: &[String], prefixes: &[&str]) -> bool {
    diffs
        .iter()
        .any(|p| prefixes.iter().any(|pre| p == pre || p.starts_with(pre)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_have_no_diffs() {
        assert!(diff_paths(&lt_proto::Settings::default()).is_empty());
    }

    #[test]
    fn nested_field_diffs_located() {
        let mut s = lt_proto::Settings::default();
        s.vad_threshold += 0.1;
        s.style.bg_color = "#123456".into();
        s.subtitle_mode.sentences = 3;
        let d = diff_paths(&s);
        assert!(d.contains(&"vad_threshold".to_string()), "{d:?}");
        assert!(d.contains(&"style.bg_color".to_string()), "{d:?}");
        assert!(d.contains(&"subtitle_mode.sentences".to_string()), "{d:?}");
        assert!(!d.contains(&"vad_mode".to_string()), "{d:?}");
    }

    #[test]
    fn model_collection_changes_detected() {
        let mut s = lt_proto::Settings::default();
        let cfg = lt_proto::ModelConfig {
            name: "my-model".into(),
            ..Default::default()
        };
        s.models.push(cfg);
        let d = diff_paths(&s);
        // 数组长度差 → 整条 models 路径
        assert!(d.contains(&"models".to_string()), "{d:?}");
    }

    #[test]
    fn model_inner_field_diff_detected() {
        let mut s = lt_proto::Settings::default();
        s.models[0].api_base = "http://localhost:9999/v1".into();
        let d = diff_paths(&s);
        assert!(d.contains(&"models[0].api_base".to_string()), "{d:?}");
    }

    #[test]
    fn any_dirty_prefix_match() {
        let d = vec!["style.bg_color".to_string(), "vad_threshold".to_string()];
        assert!(any_dirty(&d, &["style."]));
        assert!(!any_dirty(&d, &["subtitle_mode."]));
        assert!(any_dirty(&d, &["vad_mode", "vad_threshold"]));
    }
}
