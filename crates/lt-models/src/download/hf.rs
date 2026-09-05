//! HuggingFace 文件直链（/resolve/{rev}/{path}，302→CDN，支持 Range 续传）。

/// 单文件下载 URL；`endpoint` 默认 https://huggingface.co，可覆写 hf-mirror.com
pub fn file_url(endpoint: &str, repo: &str, path: &str) -> String {
    format!("{endpoint}/{repo}/resolve/main/{path}")
}
