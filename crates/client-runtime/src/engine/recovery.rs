use super::*;

impl Engine {
    pub async fn recover_unreachable_shared(
        shared: &Arc<tokio::sync::Mutex<Self>>,
    ) -> Result<Option<String>> {
        Self::recover_unreachable_cancellable(shared, None).await
    }

    pub(crate) async fn recover_unreachable_cancellable(
        shared: &Arc<tokio::sync::Mutex<Self>>,
        mut disconnected: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> Result<Option<String>> {
        let (mut job, api, settings, child, revision) = {
            let engine = shared.lock().await;
            if let Some(remote) = engine.remote.clone() {
                drop(engine);
                let result = remote.request(ControlCommand::RecoverUnreachable).await?;
                let mut engine = shared.lock().await;
                engine.apply_meta(remote.request(ControlCommand::Meta).await?)?;
                return Ok(result);
            }
            if !engine.running()
                || engine.settings.claude_protection.is_some()
                || engine.settings.proxy_mode == ProxyMode::Off
            {
                return Ok(None);
            }
            (
                engine.unreachable_recovery.begin("正在确认全部节点状态")?,
                engine.api.clone(),
                serde_json::to_value(&engine.settings)?,
                engine.network_core(),
                engine.view_revision(),
            )
        };
        if !job
            .download(
                async {
                    tokio::time::timeout(
                        Duration::from_secs(120),
                        api.confirm_all_nodes_unreachable(),
                    )
                    .await
                    .context("全部节点复测超时，已保留网络设置")?
                },
                &mut disconnected,
            )
            .await?
        {
            return Ok(None);
        }
        let engine = shared.lock().await;
        job.ensure_current(&disconnected)?;
        if !engine.running()
            || engine.network_core() != child
            || engine.view_revision() != revision
            || serde_json::to_value(&engine.settings)? != settings
            || engine.settings.claude_protection.is_some()
        {
            return Ok(None);
        }
        let message = unreachable_notice(engine.settings.proxy_mode);
        append(
            &engine.logs,
            format!("[客户端] 全部节点经两个独立目标复测仍不可达；{message}"),
        );
        Ok(Some(message.into()))
    }

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
        if super::protection_pauses_network(&self.settings) {
            append(
                &self.logs,
                "[客户端] Claude 保护期间网络发生变化，已暂停自动刷新内核出站与 DNS",
            );
            self.network_monitor.finish(true, Instant::now());
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

pub(crate) fn unreachable_notice(mode: ProxyMode) -> &'static str {
    if mode == ProxyMode::Tun {
        "全部节点不可达，TUN 仍开启。请更换节点；更新失败后会自动尝试直连。"
    } else {
        "全部节点不可达，系统代理仍开启。请更换节点；更新失败后会自动尝试直连。"
    }
}
