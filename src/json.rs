//! Minimal JSON string rendering helpers for runtime artifacts.

const EXTRA_QUOTE_CAPACITY: usize = 2;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
const HEX_SHIFTS: [u32; 4] = [12, 8, 4, 0];
const LOW_NIBBLE_MASK: u32 = 0x0f;

pub(crate) fn string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + EXTRA_QUOTE_CAPACITY);
    escaped.push('"');

    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            value if value.is_control() => push_unicode_escape(&mut escaped, value),
            value => escaped.push(value),
        }
    }

    escaped.push('"');
    escaped
}

fn push_unicode_escape(target: &mut String, value: char) {
    let code = value as u32;
    target.push_str("\\u");

    for shift in HEX_SHIFTS {
        let nibble = (code >> shift) & LOW_NIBBLE_MASK;
        target.push(hex_digit(nibble));
    }
}

fn hex_digit(nibble: u32) -> char {
    usize::try_from(nibble).map_or('0', |index| {
        HEX_DIGITS.get(index).copied().map_or('0', char::from)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn escapes_json_strings() {
        assert_eq!(
            super::string("line\n\"quoted\"\\path"),
            "\"line\\n\\\"quoted\\\"\\\\path\""
        );
        assert_eq!(
            super::string("carriage\rreturn\tindent"),
            "\"carriage\\rreturn\\tindent\""
        );
        assert_eq!(super::string("\u{0008}"), "\"\\u0008\"");
    }
}
