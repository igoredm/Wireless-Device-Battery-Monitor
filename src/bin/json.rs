//! `batteryhub-json`: one read of every supported device, printed as a JSON array on stdout, then
//! exit. No window, tray, settings changes, updater or network: made for Pitwall
//! (github.com/igoredm/pitwall), which runs it every minute and shows the levels in its widget.
//!
//! The fields match the C++ BatteryHub's `--json` (src/json_output.h) so existing readers keep
//! working; adding a field is backwards compatible.
#![cfg(windows)]

use std::io::Write;

use batteryhub::{DeviceKind, DeviceStatus};

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn kind(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Mouse => "mouse",
        DeviceKind::Headset | DeviceKind::Earbuds => "headset",
        DeviceKind::Keyboard => "keyboard",
        DeviceKind::Gamepad => "gamepad",
        DeviceKind::Phone => "phone",
        DeviceKind::Microphone | DeviceKind::Other => "other",
    }
}

fn to_json(list: &[DeviceStatus]) -> String {
    let items: Vec<String> = list
        .iter()
        .map(|s| {
            let detail = if s.detail.is_empty() { s.buds.and_then(|b| b.levels()).unwrap_or_default() } else { s.detail.clone() };
            format!(
                "{{\"key\":\"{}\",\"name\":\"{}\",\"via\":\"{}\",\"kind\":\"{}\",\"percent\":{},\"level\":\"{}\",\"detail\":\"{}\",\"charging\":{},\"full\":{},\"asleep\":{},\"connected\":{},\"millivolts\":{},\"graceSec\":{}}}",
                escape(&s.key),
                escape(&s.name),
                escape(&s.via),
                kind(s.kind),
                s.percent,
                escape(&s.level_text),
                escape(&detail),
                s.charging,
                s.full,
                s.asleep,
                s.connected,
                s.millivolts,
                s.grace_sec
            )
        })
        .collect();
    format!("[{}]\n", items.join(","))
}

fn main() {
    // English texts ("2.4 GHz receiver", "Low"...) whatever the Windows language: Pitwall translates.
    batteryhub::i18n::set_language("en");
    let text = to_json(&batteryhub::poll_all_devices());
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert_eq!(escape("a\"b\\c\nd\u{1}"), "a\\\"b\\\\c\\nd\\u0001");
        assert_eq!(escape("Fone ção"), "Fone ção");
    }

    #[test]
    fn array() {
        let s = DeviceStatus { key: "k".into(), name: "AULA X5 Ultra".into(), kind: DeviceKind::Mouse, percent: 77, ..Default::default() };
        let j = to_json(&[s]);
        assert!(j.starts_with("[{\"key\":\"k\",\"name\":\"AULA X5 Ultra\",\"via\":\"\",\"kind\":\"mouse\",\"percent\":77,"));
        assert!(j.ends_with("}]\n"));
        assert_eq!(to_json(&[]), "[]\n");
    }
}
