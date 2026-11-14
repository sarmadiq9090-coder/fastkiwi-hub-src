//! The Hub's own settings file: unreadable input falls back to defaults.

use kiwi_hub::settings::{DigitsChoice, LangChoice, Settings, ThemeChoice};

#[test]
fn defaults_and_recovery() {
    let d = Settings::default();
    assert_eq!(
        (d.theme, d.lang, d.digits, d.follow),
        (
            ThemeChoice::Auto,
            LangChoice::Auto,
            DigitsChoice::Auto,
            true
        )
    );
    assert_eq!(Settings::from_json("garbage"), d);
    assert_eq!(Settings::from_json("{\"theme\":\"neon\"}"), d);
}

#[test]
fn reads_choices_and_cleans_hidden_ids() {
    let s = Settings::from_json(
        "{\"theme\":\"midnight\",\"lang\":\"en\",\"digits\":\"latn\",\"follow\":false,\"hidden\":[\"lemon\",\"lemon\",\"../x\",\"instafind\"]}",
    );
    assert_eq!(s.theme, ThemeChoice::Midnight);
    assert_eq!(s.lang, LangChoice::En);
    assert_eq!(s.digits, DigitsChoice::Latn);
    assert!(!s.follow);
    assert_eq!(s.hidden, ["instafind", "lemon"]);
}
