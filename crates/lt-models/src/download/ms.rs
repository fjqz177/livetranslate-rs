//! ModelScope 文件直链（API v1 repo 端点；响应结构已于 M1 实测：302→文件流）。

/// 单文件下载 URL；`endpoint` 默认 https://modelscope.cn
pub fn file_url(endpoint: &str, repo: &str, path: &str) -> String {
    format!("{endpoint}/api/v1/models/{repo}/repo?Revision=master&FilePath={path}")
}
