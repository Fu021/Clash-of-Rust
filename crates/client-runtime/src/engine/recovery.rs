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
        let result = self.refresh_network().await;
        self.network_monitor.finish(result.is_ok(), Instant::now());
        match result {
            Ok(()) => append(
                &self.logs,
                "[客户端] 网络刷新完成，已刷新出站与 DNS 并清理旧连接；目标服务是否可用仍需检查",
            ),
            Err(error) => self.record_failure(&format!(
                "网络恢复未完成，将退避重试；保留原订阅与设置：{error:#}"
            )),
        }
    }

    pub(crate) async fn recover_network(&mut self) -> Result<()> {
        anyhow::ensure!(self.running() && !self.is_remote(), "当前用户内核未运行");
        append(&self.logs, "[客户端] 正在手动刷新内核出站与 DNS");
        match self.refresh_network().await {
            Ok(()) => {
                append(
                    &self.logs,
                    "[客户端] 手动网络刷新完成；目标服务是否可用仍需检查",
                );
                Ok(())
            }
            Err(error) => {
                self.record_failure(&format!("手动网络刷新未完成：{error:#}"));
                Err(error.context("网络刷新未完成，部分步骤可能已经生效，请检查状态"))
            }
        }
    }

    async fn refresh_network(&mut self) -> Result<()> {
        let payload = read_runtime_config(&self.store.runtime().join("config.yaml"))?;
        self.api.refresh_network(&payload).await?;
        self.healthy().await?;
        self.last_totals = None;
        self.node_root.lock().expect("header route lock").take();
        Ok(())
    }
}
