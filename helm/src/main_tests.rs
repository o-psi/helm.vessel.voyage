use super::*;
#[test]
fn diagnostics_strip_terminal_controls_and_bidi_without_damaging_unicode() {
    let dangerous = "\x00\x1b[31msecret\x7f\u{80}\u{9f}\u{202a}α\u{202e}\u{2066}λ\u{2069}\n";
    assert_eq!(safe_diagnostic(dangerous), "[31msecretαλ");
    assert_eq!(safe_diagnostic("日本語 café 👩‍💻"), "日本語 café 👩‍💻");
    assert_eq!(safe_diagnostic(""), "");
}
