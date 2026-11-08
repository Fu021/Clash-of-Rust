use crate::args::Args;
use serde_json::Value;

pub(crate) use clash_client_runtime::platform::host_network::{
    holder_summary, notice, with_proxy_context,
};

pub(crate) async fn observe(args: &Args) -> Value {
    if args.flag("all-users") {
        clash_client_runtime::platform::host_network::observe_all_users().await
    } else {
        clash_client_runtime::platform::host_network::observe().await
    }
}
