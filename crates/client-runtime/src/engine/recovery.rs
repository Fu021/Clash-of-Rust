use super::*;

impl Engine {
    pub(crate) fn network_core(&self) -> Option<u32> {
        if self.is_remote() {
            return None;
        }
        self.child.as_ref().and_then(Child::id)
    }

    pub(crate) async fn network_changed(&mut self, routes: platform::network::Routes) {
        if !self.network_monitor.observe(routes, Instant::now()) {
            return;
        }
        append(
            &self.logs,
            "[客户端] 网络已恢复或默认路由已变化，正在刷新内核出站与 DNS",
        );
        let result = async {
            let payload = read_runtime_config(&self.store.runtime().join("config.yaml"))?;
            self.api.refresh_network(&payload).await?;
            self.healthy().await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.network_monitor.finish(result.is_ok(), Instant::now());
        match result {
            Ok(()) => {
                self.last_totals = None;
                self.node_root.lock().expect("header route lock").take();
                append(
                    &self.logs,
                    "[客户端] 网络恢复完成，已刷新出站与 DNS 并清理旧连接，保留节点、模式和保护设置",
                );
            }
            Err(error) => self.record_failure(&format!(
                "网络恢复未完成，将退避重试；保留原订阅与设置：{error:#}"
            )),
        }
    }
}
