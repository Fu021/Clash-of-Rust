//! Lightweight protection readiness shared by persistent GUI chrome.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum State {
    #[default]
    Off = 0,
    Active = 1,
    Checking = 2,
    Error = 3,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub state: State,
    pub message: String,
}

impl Status {
    pub fn observe(
        enabled: bool,
        changing: bool,
        running: bool,
        tun: bool,
        guard: Option<(bool, &str)>,
    ) -> Self {
        let (state, message) = if changing {
            (State::Checking, "正在切换保护，等待接管验证完成")
        } else if !enabled {
            return Self::default();
        } else if !running {
            (State::Error, "内核未运行，Claude 保护尚未生效")
        } else if !tun {
            (State::Error, "TUN 未连接，Claude 保护接管未就绪")
        } else {
            match guard {
                Some((_, error)) if !error.is_empty() => (State::Error, error),
                Some((true, _)) => (State::Active, ""),
                _ => (State::Checking, "正在确认保护连接与实时拦截状态"),
            }
        };
        Self {
            state,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_and_running_alone_do_not_imply_verified_protection() {
        assert_eq!(
            Status::observe(false, false, false, false, Some((false, "old error"))),
            Status::default()
        );
        for (changing, running, tun, guard, expected) in [
            (true, false, false, None, State::Checking),
            (false, false, false, None, State::Error),
            (false, true, false, Some((true, "")), State::Error),
            (false, true, true, None, State::Checking),
            (false, true, true, Some((false, "")), State::Checking),
            (false, true, true, Some((true, "")), State::Active),
            (
                false,
                true,
                true,
                Some((true, "DNS 接管失败")),
                State::Error,
            ),
            (
                false,
                true,
                true,
                Some((false, "DNS 接管失败")),
                State::Error,
            ),
        ] {
            let status = Status::observe(true, changing, running, tun, guard);
            assert_eq!(status.state, expected);
            assert_eq!(status.message.is_empty(), expected == State::Active);
        }
        assert_eq!(
            Status::observe(false, true, true, true, None).state,
            State::Checking
        );
    }
}
