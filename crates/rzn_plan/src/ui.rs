/// UI/console helpers shared by CLI-style flows.
///
/// In JSON mode (`RZN_JSON=1`), stdout must remain machine-readable for downstream
/// consumers (SDKs, desktop apps). These helpers suppress human-oriented prints.
pub fn json_mode() -> bool {
    std::env::var("RZN_JSON")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

#[macro_export]
macro_rules! ui_println {
    ($($arg:tt)*) => {{
        if !$crate::ui::json_mode() {
            println!($($arg)*);
        }
    }};
}
