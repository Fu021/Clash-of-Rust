use anyhow::{Context, Result, ensure};
use objc2::{
    AnyThread, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationSettings,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use std::sync::{Mutex, OnceLock};

define_class!(
    // No instance state; this delegate can receive system callbacks on any thread.
    #[unsafe(super = NSObject)]
    struct NotificationDelegate;
    unsafe impl NSObjectProtocol for NotificationDelegate {}
    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // macOS otherwise suppresses banners while our window is in front.
            completion
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }
    }
);
// SAFETY: the class has no mutable ivars, and NSObject reference counting is thread safe.
unsafe impl Send for NotificationDelegate {}
unsafe impl Sync for NotificationDelegate {}
static DELEGATE: OnceLock<Retained<NotificationDelegate>> = OnceLock::new();
static AUTHORIZATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type Completion = tokio::sync::oneshot::Receiver<Result<(), String>>;
fn error_text(error: *mut NSError) -> Option<String> {
    // Only inspect NSError during Apple's callback, while it is borrowed and alive.
    unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Permission {
    Allowed,
    RequestedDenied,
    Settings,
}
#[derive(Debug, Clone, Copy)]
struct Settings {
    authorization: isize,
    alert: bool,
    center: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Ready,
    Request,
    Settings,
}
fn action(settings: Settings) -> Action {
    match settings.authorization {
        0 => Action::Request,
        2..=4 if settings.alert || settings.center => Action::Ready,
        _ => Action::Settings,
    }
}
fn center() -> Result<Retained<UNUserNotificationCenter>> {
    ensure!(
        NSBundle::mainBundle().bundleIdentifier().is_some(),
        "macOS 系统通知需要从 Clash of Rust.app 启动"
    );
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate =
        DELEGATE.get_or_init(|| unsafe { msg_send![NotificationDelegate::alloc(), init] });
    center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
    Ok(center)
}
fn query() -> Result<tokio::sync::oneshot::Receiver<Settings>> {
    let center = center()?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let callback =
        block2::RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
            // Apple lends the object for the duration of this completion only.
            let settings = unsafe { settings.as_ref() };
            let state = Settings {
                authorization: settings.authorizationStatus().0,
                alert: settings.alertSetting().0 == 2,
                center: settings.notificationCenterSetting().0 == 2,
            };
            if let Some(sender) = sender.lock().ok().and_then(|mut s| s.take()) {
                let _ = sender.send(state);
            }
        });
    center.getNotificationSettingsWithCompletionHandler(&callback);
    Ok(receiver)
}
fn request() -> Result<tokio::sync::oneshot::Receiver<Result<bool, String>>> {
    let center = center()?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let callback = block2::RcBlock::new(move |granted: Bool, error: *mut NSError| {
        let result = error_text(error).map_or(Ok(granted.as_bool()), Err);
        if let Some(sender) = sender.lock().ok().and_then(|mut s| s.take()) {
            let _ = sender.send(result);
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert,
        &callback,
    );
    Ok(receiver)
}
pub(super) async fn permission() -> Result<Permission> {
    let _request = AUTHORIZATION.lock().await;
    let settings = objc2::rc::autoreleasepool(|_| query())?;
    let settings = settings.await.context("macOS 通知设置查询未完成")?;
    match action(settings) {
        Action::Ready => Ok(Permission::Allowed),
        Action::Settings => Ok(Permission::Settings),
        Action::Request => {
            let request = objc2::rc::autoreleasepool(|_| request())?;
            let granted = request
                .await
                .context("macOS 通知授权未完成")?
                .map_err(anyhow::Error::msg)?;
            Ok(if granted {
                Permission::Allowed
            } else {
                Permission::RequestedDenied
            })
        }
    }
}
fn deliver(title: &str, message: &str) -> Completion {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(message));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&uuid::Uuid::new_v4().to_string()),
        &content,
        None,
    );
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let callback = block2::RcBlock::new(move |error: *mut NSError| {
        let result = error_text(error).map_or(Ok(()), Err);
        if let Some(sender) = sender.lock().ok().and_then(|mut s| s.take()) {
            let _ = sender.send(result);
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
    receiver
}
pub(super) async fn initialize() -> Result<()> {
    if permission().await? != Permission::Allowed {
        return Err(super::PermissionDenied.into());
    }
    Ok(())
}
pub(super) async fn show(title: &str, message: &str) -> Result<()> {
    initialize().await?;
    let delivered = objc2::rc::autoreleasepool(|_| deliver(title, message));
    delivered
        .await
        .context("macOS 通知提交未完成")?
        .map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permission_policy_requests_only_undetermined_authorization() {
        let settings = |authorization, alert, center| Settings {
            authorization,
            alert,
            center,
        };
        assert_eq!(action(settings(0, false, false)), Action::Request);
        assert_eq!(action(settings(1, false, false)), Action::Settings);
        assert_eq!(action(settings(2, true, true)), Action::Ready);
        assert_eq!(action(settings(2, false, true)), Action::Ready);
        assert_eq!(action(settings(2, false, false)), Action::Settings);
        assert_eq!(action(settings(3, false, true)), Action::Ready);
        assert_eq!(action(settings(4, true, true)), Action::Ready);
        assert_eq!(action(settings(999, true, true)), Action::Settings);
    }
}
