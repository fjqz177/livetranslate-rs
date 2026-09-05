// lt-models: 模型注册表 / 缓存探测 / 双 hub 下载器 / 设置持久化
pub mod cache;
pub mod paths;
pub mod registry;
pub mod settings_io;

/// 测试专用：LIVETRANSLATE_CONFIG_DIR 是进程级环境变量，所有用到它的
/// 测试必须串行（否则并行测试互相覆盖配置目录，污染真实 ~/.config）。
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
