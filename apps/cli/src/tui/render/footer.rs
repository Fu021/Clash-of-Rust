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
        || state.updates.pending.is_some()
        || state
            .rules
            .as_ref()
            .is_some_and(|rules| rules.pending.is_some())
}

pub(super) fn compact_help(state: &State) -> &'static str {
    let language = state.language;
    if state.entering_text() {
        language.text("Ctrl+C 退出 · Esc 返回", "Ctrl+C quit · Esc back")
    } else if state.page == Page::Home && state.form.is_none() && state.updates.pending.is_none() {
        language.text("Enter 操作 · q 退出", "Enter action · q quit")
    } else {
        language.text("q 退出 · Esc 返回", "q quit · Esc back")
    }
}

fn help(state: &State) -> &'static str {
    let language = state.language;
    if state.execution.is_some() || state.confirmation.is_some() || state.updates.pending.is_some()
    {
        language.text(
            "Enter 确认 · Esc 取消 · q 退出",
            "Enter confirm · Esc cancel · q quit",
        )
    } else if state.form.as_ref().is_some_and(|form| form.editing) {
        language.text(
            "输入中 · Enter 保留草稿 · Ctrl+U 清空 · Esc 撤销本项 · Ctrl+C 退出",
            "Enter keep draft · Ctrl+U clear · Esc undo field · Ctrl+C quit",
        )
    } else if state.searching {
        language.text(
            "Enter 完成输入 · Ctrl+U 清空 · Esc 取消 · Ctrl+C 退出",
            "Enter finish · Ctrl+U clear · Esc cancel · Ctrl+C quit",
        )
    } else if state.page == Page::Rules
        && !state.shortcuts_open
        && let Some(rules) = &state.rules
    {
        if rules.pending.is_some() {
            "Enter 确认保存 · Esc 取消 · q 退出"
        } else if rules.input.as_ref().is_some_and(|input| input.editing) {
            "输入中 · Enter 检查并保存 · Ctrl+U 清空 · Esc 保留输入 · Ctrl+C 退出"
        } else if rules.input.is_some() {
            "Enter 编辑 · Esc 返回 · q 退出"
        } else if rules.menu.is_some() {
            "↑↓ 选择操作 · Enter 打开/确认 · Esc 返回 · q 退出"
        } else {
            "↑↓ 选择 · Enter 打开 · / 搜索 · r 刷新 · q 退出"
        }
    } else if let Some(editor) = &state.fake_ip {
        if editor.pending.is_some() {
            "Enter 确认保存 · Esc 取消 · q 退出"
        } else if editor.editing {
            "输入中 · Enter 保存新增项 · Ctrl+U 清空 · Esc 保留输入 · Ctrl+C 退出"
        } else {
            "↑↓ 选择 · Enter 新增/停用/恢复 · Esc 返回 · q 退出"
        }
    } else if let Some(form) = &state.form {
        if form
            .fields
            .get(form.selected)
            .is_some_and(|field| !field.choices.is_empty())
        {
            if form.setting.is_some() {
                language.text(
                    "↑↓ 选择 · ←→ 调整 · Enter 确认当前值 · Esc 返回 · q 退出",
                    "↑↓ select · ←→ change · Enter review · Esc back · q quit",
                )
            } else {
                language.text(
                    "↑↓ 选择 · ←→ 调整 · Enter 下一项 · Esc 返回 · q 退出",
                    "↑↓ select · ←→ change · Enter next · Esc back · q quit",
                )
            }
        } else {
            language.text(
                "↑↓ 选择 · Enter 编辑/确认 · Esc 返回 · q 退出",
                "↑↓ select · Enter edit or review · Esc back · q quit",
            )
        }
    } else if state.connection.is_some() {
        "↑↓ 详情 · d 解析 · x 关闭 · Esc 返回 · q 退出"
    } else if state.page == Page::Network {
        "←→ 切换工具 · o 参数 · e 导出 · / 输入 · Enter 检查 · ↑↓ 结果 · Esc 取消 · q 退出"
    } else if state.page == Page::Nodes {
        "↑↓ 选择 · Enter 打开 · s 排序 · / 搜索 · Esc 返回 · q 退出"
    } else if state.page == Page::Home {
        "↑↓ 选择 · Enter 操作 · r 刷新 · q 退出"
    } else {
        language.text(
            "↑↓ 选择 · Enter 打开 · / 搜索 · Esc 返回 · q 退出",
            "↑↓ select · Enter open · / search · Esc back · q quit",
        )
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

pub(super) fn lines(
    state: &State,
    width: usize,
    height: usize,
    header_height: usize,
    spacious: bool,
) -> Vec<Line> {
    let line = |text: &str, tone| Line {
        text: fit(text, width),
        tone,
    };
    let (notice, tone) = super::notice(state);
    if height < 10 {
        return vec![if has_notice(state) {
            line(&notice, tone)
        } else {
            line(compact_help(state), Tone::Muted)
        }];
    }
    let budget = height.saturating_sub(header_height + 3 + if spacious { 2 } else { 0 });
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
    let mut lines = Vec::new();
    if spacious {
        lines.push(line(&"─".repeat(width), Tone::Muted));
    }
    lines.extend(
        notice
            .iter()
            .map(|text| line(text, tone))
            .chain(help.iter().map(|text| line(text, Tone::Muted))),
    );
    if spacious {
        lines.push(line(
            &if state.error.is_some() {
                "诊断：cor diagnose --output cor-diagnose.json".into()
            } else {
                format!(
                    "v{} · 本机后台 · 每秒采样，变化时绘制",
                    clash_client_core::VERSION
                )
            },
            Tone::Muted,
        ));
    }
    lines
}
