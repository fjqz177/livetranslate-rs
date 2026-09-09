//! ModelScope 文件直链（API v1 repo 端点；响应结构已于 M1 实测：302→文件流）。

/// 单文件下载 URL；`endpoint` 默认 https://modelscope.cn。
/// DL-6/F11（docs/archive/download-overhaul.md）：经 `Url::parse_with_params` 正确
/// 编码 Revision/FilePath——文件名含空格/中文/特殊字符不再裸拼。
pub fn file_url(endpoint: &str, repo: &str, path: &str) -> String {
    reqwest::Url::parse_with_params(
        &format!("{endpoint}/api/v1/models/{repo}/repo"),
        &[("Revision", "master"), ("FilePath", path)],
    )
    .expect("MS repo URL 构造失败：endpoint/repo 含非法字符")
    .into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn url_matches_legacy_shape_and_encodes_path() {
        let url = super::file_url("https://modelscope.cn", "iic/SenseVoiceSmall", "tokens.txt");
        assert_eq!(
            url,
            "https://modelscope.cn/api/v1/models/iic/SenseVoiceSmall/repo?Revision=master&FilePath=tokens.txt"
        );
        // 特殊字符经表单编码（Url query 约定：空格→+，非 ASCII→%XX），不再裸拼
        let url = super::file_url("https://modelscope.cn", "iic/M", "a b.txt");
        assert!(url.contains("FilePath=a+b.txt"), "url={url}");
        assert!(!url[url.find('?').unwrap()..].contains(' '), "url={url}");
    }
}
