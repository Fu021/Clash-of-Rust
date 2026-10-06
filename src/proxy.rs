use anyhow::Result;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    Off,
    System,
    Tun,
}

pub(crate) trait Backend {
    async fn tun_enabled(&self) -> Result<bool>;
    fn system_enabled(&self) -> Result<bool>;
    async fn set_tun(&self, enabled: bool) -> Result<()>;
    fn set_system(&self, enabled: bool) -> Result<()>;
}

async fn apply(backend: &impl Backend, mode: ProxyMode) -> Result<()> {
    let tun = backend.tun_enabled().await?;
    if mode != ProxyMode::Tun && tun {
        backend.set_tun(false).await?;
    }
    if mode != ProxyMode::System {
        backend.set_system(false)?;
    }
    match mode {
        ProxyMode::Tun if !tun => backend.set_tun(true).await?,
        ProxyMode::System if !backend.system_enabled()? => backend.set_system(true)?,
        _ => {}
    }
    Ok(())
}

pub(crate) async fn select(backend: &impl Backend, mode: ProxyMode) -> Result<()> {
    let previous = if backend.tun_enabled().await? {
        ProxyMode::Tun
    } else if backend.system_enabled()? {
        ProxyMode::System
    } else {
        ProxyMode::Off
    };
    if let Err(error) = apply(backend, mode).await {
        return match apply(backend, previous).await {
            Ok(()) => Err(error.context("代理模式切换失败，已恢复原模式")),
            Err(rollback) => {
                Err(error.context(format!("代理模式切换失败，恢复原模式也失败：{rollback:#}")))
            }
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake(Mutex<(bool, bool, bool, Vec<&'static str>)>);
    impl Backend for Fake {
        async fn tun_enabled(&self) -> Result<bool> {
            Ok(self.0.lock().unwrap().0)
        }
        fn system_enabled(&self) -> Result<bool> {
            Ok(self.0.lock().unwrap().1)
        }
        async fn set_tun(&self, enabled: bool) -> Result<()> {
            let mut state = self.0.lock().unwrap();
            assert!(!enabled || !state.1, "TUN must never overlap system proxy");
            state.0 = enabled;
            state.3.push(if enabled { "tun on" } else { "tun off" });
            if enabled && state.2 {
                state.2 = false;
                anyhow::bail!("simulated controller failure after applying TUN");
            }
            Ok(())
        }
        fn set_system(&self, enabled: bool) -> Result<()> {
            let mut state = self.0.lock().unwrap();
            assert!(!enabled || !state.0, "System proxy must never overlap TUN");
            state.1 = enabled;
            state
                .3
                .push(if enabled { "system on" } else { "system off" });
            Ok(())
        }
    }

    #[tokio::test]
    async fn switches_both_ways_and_off_without_overlap() {
        let backend = Fake(Mutex::new((false, true, false, Vec::new())));
        select(&backend, ProxyMode::Tun).await.unwrap();
        select(&backend, ProxyMode::System).await.unwrap();
        select(&backend, ProxyMode::Off).await.unwrap();
        let state = backend.0.lock().unwrap();
        assert_eq!((state.0, state.1), (false, false));
        assert_eq!(
            state.3,
            ["system off", "tun on", "tun off", "system on", "system off"]
        );
    }

    #[tokio::test]
    async fn failed_tun_switch_rolls_back_even_after_partial_success() {
        let backend = Fake(Mutex::new((false, true, true, Vec::new())));
        assert!(select(&backend, ProxyMode::Tun).await.is_err());
        let state = backend.0.lock().unwrap();
        assert_eq!((state.0, state.1), (false, true));
        assert_eq!(state.3, ["system off", "tun on", "tun off", "system on"]);
    }

    struct FailingSystem(Fake);
    impl Backend for FailingSystem {
        async fn tun_enabled(&self) -> Result<bool> {
            self.0.tun_enabled().await
        }
        fn system_enabled(&self) -> Result<bool> {
            self.0.system_enabled()
        }
        async fn set_tun(&self, enabled: bool) -> Result<()> {
            self.0.set_tun(enabled).await
        }
        fn set_system(&self, enabled: bool) -> Result<()> {
            {
                let mut state = self.0.0.lock().unwrap();
                if enabled && state.2 {
                    assert!(!state.0);
                    state.1 = true;
                    state.2 = false;
                    state.3.push("system on");
                    anyhow::bail!("simulated OS proxy failure after partial application");
                }
            }
            self.0.set_system(enabled)
        }
    }

    #[tokio::test]
    async fn failed_system_switch_restores_tun_without_overlap() {
        let backend = FailingSystem(Fake(Mutex::new((true, false, true, Vec::new()))));
        assert!(select(&backend, ProxyMode::System).await.is_err());
        let state = backend.0.0.lock().unwrap();
        assert_eq!((state.0, state.1), (true, false));
        assert_eq!(state.3, ["tun off", "system on", "system off", "tun on"]);
    }
}
