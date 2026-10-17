use anyhow::{Context, Result, ensure};
use objc2::{
    AnyThread, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
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
fn authorize() -> Result<Completion> {
    ensure!(
        NSBundle::mainBundle().bundleIdentifier().is_some(),
        "macOS 系统通知需要从 Clash of Rust.app 启动"
    );
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate =
        DELEGATE.get_or_init(|| unsafe { msg_send![NotificationDelegate::alloc(), init] });
    center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let callback = block2::RcBlock::new(move |granted: Bool, error: *mut NSError| {
        let result = if let Some(error) = error_text(error) {
            Err(error)
        } else if granted.as_bool() {
            Ok(())
        } else {
            Err("请在系统设置的通知中允许 Clash of Rust 发送通知".into())
        };
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
    let _request = AUTHORIZATION.lock().await;
    let permission = objc2::rc::autoreleasepool(|_| authorize())?;
    permission
        .await
        .context("macOS 通知授权未完成")?
        .map_err(anyhow::Error::msg)
}
pub(super) async fn show(title: &str, message: &str) -> Result<()> {
    initialize().await?;
    let delivered = objc2::rc::autoreleasepool(|_| deliver(title, message));
    delivered
        .await
        .context("macOS 通知提交未完成")?
        .map_err(anyhow::Error::msg)
}
