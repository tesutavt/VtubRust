//! UI language. Russian strings in the code are the keys; English translations live in
//! `i18n_en.rs`. `tr()` returns the key itself for Russian or when no translation exists.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static EN: AtomicBool = AtomicBool::new(false);

pub fn set_language(code: &str) {
    EN.store(code == "en", Ordering::Relaxed);
}

pub fn is_en() -> bool {
    EN.load(Ordering::Relaxed)
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| include!("i18n_en.rs").iter().copied().collect())
}

pub fn tr(s: &str) -> &str {
    if !is_en() {
        return s;
    }
    table().get(s).copied().unwrap_or(s)
}
