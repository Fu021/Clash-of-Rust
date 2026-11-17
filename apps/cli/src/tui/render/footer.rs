use super::{
    super::model::{Page, State},
    Line, Tone, fit, wrap,
};
use unicode_width::UnicodeWidthStr;

pub(super) fn has_notice(state: &State) -> bool {
    state.execution.is_some()
        || state.confirmation.is_some()
        || state.error.is_some()
        || state.message.is_some()
        || state.saving
}

fn help(state: &State) -> &'static str {
    if state.execution.is_some() || state.confirmation.is_some() {
        "Enter 确认 · Esc 取消 · q 退出"
    } else if state.form.as_ref().is_some_and(|form| form.editing) {
        "Enter 保留草稿 · Ctrl+U 清空 · Esc 撤销本项 · Ctrl+C 退出"
    } else if state.searching {
        "Enter 完成输入 · Ctrl+U 清空 · Esc 取消 · Ctrl+C 退出"
    } else if state.form.is_some() {
        "↑↓ 选择 · ←→ 切换 · Enter 编辑/确认 · Esc 返回 · q 退出"
    } else if state.connection.is_some() {
        "↑↓ 详情 · d 解析 · x 关闭 · Esc 返回 · q 退出"
    } else if state.page == Page::Network {
        "[ ] 工具 · / 输入 · Enter 检查 · Esc 取消 · q 退出"
    } else if state.page == Page::Nodes {
        "↑↓ 选择 · Enter 打开 · s 排序 · / 搜索 · Esc 返回 · q 退出"
    } else {
        "↑↓ 选择 · Enter 打开 · / 搜索 · Esc 返回 · q 退出"
    }
}

fn wrap_help(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for action in text.split(" · ") {
        if !current.is_empty() && current.width() + 3 + action.width() > width {
            lines.push(std::mem::take(&mut current));
        }
        if action.width() > width {
            lines.extend(wrap(action, width));
        } else {
            if !current.is_empty() {
                current.push_str(" · ");
            }
            current.push_str(action);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

pub(super) fn lines(state: &State, width: usize, height: usize, spacious: bool) -> Vec<Line> {
    let line = |text: &str, tone| Line {
        text: fit(text, width),
        tone,
    };
    let (notice, tone) = super::notice(state);
    if spacious {
        return vec![
            line(&"─".repeat(width), Tone::Muted),
            line(&notice, tone),
            line(help(state), Tone::Muted),
            line(
                &if state.error.is_some() {
                    "诊断：cor diagnose --output cor-diagnose.json".into()
                } else {
                    format!(
                        "v{} · 本机后台 · 每秒采样，变化时绘制",
                        clash_client_core::VERSION
                    )
                },
                Tone::Muted,
            ),
        ];
    }
    if height < 10 {
        return vec![if has_notice(state) {
            line(&notice, tone)
        } else if state.entering_text() {
            line("Ctrl+C 退出 · Esc 返回", Tone::Muted)
        } else {
            line("q 退出 · Esc 返回", Tone::Muted)
        }];
    }
    let budget = height.saturating_sub(4);
    let mut help = wrap_help(help(state), width);
    help.truncate(budget.saturating_sub(1));
    let mut notice = wrap(
        if has_notice(state) {
            &notice
        } else {
            "退出界面保留后台和代理"
        },
        width,
    );
    let available = budget.saturating_sub(help.len());
    if notice.len() > available {
        notice.truncate(available);
        if let Some(last) = notice.last_mut() {
            last.push('…');
        }
    }
    notice
        .iter()
        .map(|text| line(text, tone))
        .chain(help.iter().map(|text| line(text, Tone::Muted)))
        .collect()
}
