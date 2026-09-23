//! 契约面纯度守护（ADR-21：原 scripts/check_guards.ps1 禁令 5 迁入）。
//!
//! events.rs 是命令/事件契约的唯一面孔，禁止 serde_json::Value 裸载荷——
//! 数据透传必须走 settings.rs 的显式契约字段（ModelConfig.overrides/extra_body，
//! D-82/D-85），否则契约消费者无法穷尽匹配、旁路即 P1（AGENTS §3）。

const EVENTS: &str = include_str!("../src/events.rs");

#[test]
fn events_rs_carries_no_raw_json() {
    let hits: Vec<usize> = EVENTS
        .lines()
        .enumerate()
        .filter(|(_, ln)| ln.contains("serde_json::Value"))
        .map(|(i, _)| i + 1)
        .collect();
    assert!(
        hits.is_empty(),
        "events.rs 第 {hits:?} 行出现 serde_json::Value——命令/事件载荷必须强类型（AGENTS §3；\
         数据透传走 settings.rs 显式契约字段）"
    );
}
