use clash_mobile_runtime::core::{config::Language, error_message, update};
use std::borrow::Cow;

pub(crate) fn check_failure(error: &str, language: Language) -> Cow<'_, str> {
    update::localize_check_error(error, language)
}

pub(crate) fn download_failure(error: &str, language: Language) -> String {
    let detail = error_message::localize(error, language);
    format!("{}{detail}", language.text("更新失败：", "Update failed: "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_update_errors_follow_the_current_language() {
        let check =
            "检查更新失败（已尝试 2 次）：系统网络：更新服务器返回 HTTP 503 Service Unavailable";
        let download = "下载更新失败：直连：安装包大小或 SHA256 校验失败";
        assert_eq!(check_failure(check, Language::SimplifiedChinese), check);
        assert_eq!(
            check_failure(check, Language::English),
            "Update check failed after 2 attempts: System network: The update server returned HTTP 503 Service Unavailable"
        );
        assert_eq!(
            download_failure(download, Language::English),
            "Update failed: Update download failed: Direct: Package size or SHA256 verification failed"
        );
        assert_eq!(
            download_failure(download, Language::SimplifiedChinese),
            format!("更新失败：{download}")
        );
    }
}
