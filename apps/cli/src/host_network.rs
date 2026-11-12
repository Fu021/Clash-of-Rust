use crate::args::Args;
use serde_json::Value;

pub(crate) use clash_client_runtime::platform::host_network::{
    holder_summary, notice, with_proxy_context, with_settings_context,
};

pub(crate) async fn observe(args: &Args) -> Value {
    if args.flag("all-users") {
        clash_client_runtime::platform::host_network::observe_all_users().await
    } else {
        clash_client_runtime::platform::host_network::observe().await
    }
}

pub(crate) async fn diagnose(args: &Args) -> Value {
    use clash_client_runtime::platform::host_network::{HolderScope, diagnose};
    diagnose(if args.flag("all-users") {
        HolderScope::AllUsers
    } else {
        HolderScope::CurrentUser
    })
    .await
}
