//! 运行时文案切换（中文 / 英文）。
//!
//! 托盘菜单、桌面通知和命令报错都由 Rust 生成，必须和前端共用同一种语言。
//! 进程内只有一个窗口，语言是全局状态就够了，不必把 `locale` 一路透传进每个函数。

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_locale(value: &str) {
    ENGLISH.store(value.eq_ignore_ascii_case("en"), Ordering::Relaxed);
}

pub fn is_en() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

pub fn current() -> &'static str {
    if is_en() { "en" } else { "zh" }
}

/// 静态文案：中文在前、英文在后，按当前语言取一个。
pub fn tr<'a>(zh: &'a str, en: &'a str) -> &'a str {
    if is_en() { en } else { zh }
}
