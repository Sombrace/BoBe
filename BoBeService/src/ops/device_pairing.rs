use qrcodegen::{QrCode, QrCodeEcc};
use serde_json::Value;

const QUIET_ZONE: i32 = 4;
const DARK_ON_LIGHT: &str = "\x1b[38;5;16;48;5;231m";
const RESET: &str = "\x1b[0m";

pub(crate) fn spaced(code: &str) -> String {
    match (code.get(..3), code.get(3..)) {
        (Some(first), Some(rest)) if !rest.is_empty() => format!("{first} {rest}"),
        _ => code.to_owned(),
    }
}

pub(crate) fn render(report: &Value) -> String {
    let code = spaced(
        report
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let minutes = report
        .get("expires_in_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(300)
        / 60;
    let Some(link) = report.get("link").and_then(Value::as_str) else {
        return format!("Pairing code: {code}\nExpires in {minutes} minutes.");
    };
    let origins = report
        .get("origins")
        .and_then(Value::as_array)
        .map(|origins| origins.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut lines = vec!["Scan with BoBe on your iPhone.".to_owned(), String::new()];
    match terminal_qr(link) {
        Some(qr) => lines.extend(qr),
        None => lines.push(link.to_owned()),
    }
    lines.push(String::new());
    match origins.split_first() {
        Some((first, rest)) => {
            lines.push(format!("Or type the code {code} and the address {first}."));
            if !rest.is_empty() {
                lines.push(format!("Other addresses: {}", rest.join(", ")));
            }
        }
        None => lines.push(format!("Or type the code {code}.")),
    }
    lines.push(format!("The code expires in {minutes} minutes."));
    lines.join("\n")
}

fn terminal_qr(text: &str) -> Option<Vec<String>> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Low).ok()?;
    let edge = qr.size() + QUIET_ZONE;
    Some(
        (-QUIET_ZONE..edge)
            .step_by(2)
            .map(|y| {
                let cells: String = (-QUIET_ZONE..edge)
                    .map(|x| match (qr.get_module(x, y), qr.get_module(x, y + 1)) {
                        (false, false) => ' ',
                        (true, false) => '\u{2580}',
                        (false, true) => '\u{2584}',
                        (true, true) => '\u{2588}',
                    })
                    .collect();
                format!("{DARK_ON_LIGHT}{cells}{RESET}")
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "bobe://pair?v=1&c=012345&p=q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s\
                        &o=192.168.1.5:8766&o=bobe.example.net:8766";

    fn visible(row: &str) -> &str {
        row.strip_prefix(DARK_ON_LIGHT)
            .and_then(|row| row.strip_suffix(RESET))
            .unwrap()
    }

    #[test]
    fn a_mac_code_prints_as_the_code_and_its_expiry() {
        let report = serde_json::json!({
            "code": "012345",
            "expires_in_seconds": 300,
            "kind": "mac_app",
            "label": "Mac",
        });
        assert_eq!(
            render(&report),
            "Pairing code: 012 345\nExpires in 5 minutes."
        );
    }

    #[test]
    fn the_qr_code_is_dark_on_light_with_a_four_module_quiet_zone() {
        let qr = QrCode::encode_text(LINK, QrCodeEcc::Low).unwrap();
        let size = usize::try_from(qr.size()).unwrap();
        let rows = terminal_qr(LINK).unwrap();
        let width = size + 8;
        assert_eq!(rows.len(), width.div_ceil(2));
        for row in &rows {
            assert_eq!(visible(row).chars().count(), width);
        }
        assert!(visible(&rows[0]).chars().all(|cell| cell == ' '));
        assert!(visible(&rows[1]).chars().all(|cell| cell == ' '));
        assert!(
            visible(&rows[rows.len() - 1])
                .chars()
                .all(|cell| cell == ' ')
        );
        let finder: String = visible(&rows[2]).chars().take(12).collect();
        assert_eq!(
            finder,
            "    \u{2588}\u{2580}\u{2580}\u{2580}\u{2580}\u{2580}\u{2588} "
        );
        let right: String = visible(&rows[2]).chars().skip(width - 12).collect();
        assert_eq!(
            right,
            " \u{2588}\u{2580}\u{2580}\u{2580}\u{2580}\u{2580}\u{2588}    "
        );
        assert!(rows.iter().all(|row| row.starts_with(DARK_ON_LIGHT)));
    }

    #[test]
    fn a_phone_code_prints_the_qr_code_then_the_typed_fallback() {
        let report = serde_json::json!({
            "code": "012345",
            "expires_in_seconds": 300,
            "kind": "phone",
            "label": "iPhone",
            "link": LINK,
            "pin": "q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s",
            "origins": ["192.168.1.5:8766", "bobe.example.net:8766"],
        });
        let rendered = render(&report);
        let qr_rows = rendered
            .lines()
            .filter(|line| line.starts_with(DARK_ON_LIGHT))
            .count();
        assert_eq!(qr_rows, terminal_qr(LINK).unwrap().len());
        let fallback: Vec<&str> = rendered
            .lines()
            .skip_while(|line| !line.starts_with("Or type"))
            .collect();
        assert_eq!(
            fallback,
            [
                "Or type the code 012 345 and the address 192.168.1.5:8766.",
                "Other addresses: bobe.example.net:8766",
                "The code expires in 5 minutes.",
            ]
        );
        assert!(!rendered.contains("q6ur"));
    }
}
