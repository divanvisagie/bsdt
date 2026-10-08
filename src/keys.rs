//! Turning key names and text into QEMU key codes (`qcode`s) for
//! `bsdt key` and `bsdt type`. Text assumes a US keyboard layout.

use anyhow::{Result, bail};

/// Parse a chord such as `super+shift+t` (or `ctrl-alt-f2`) into key codes,
/// in the order they are pressed.
pub fn chord(spec: &str) -> Result<Vec<String>> {
    let names: Vec<&str> = spec.split(['+', '-']).collect();
    if names.iter().any(|n| n.is_empty()) {
        bail!("invalid key {spec:?}; join keys with + or -, and use \"minus\" or \"plus\" for those keys");
    }
    Ok(names.iter().map(|name| code(name)).collect())
}

fn code(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let code = match lower.as_str() {
        "super" | "meta" | "win" | "cmd" | "mod4" | "logo" => "meta_l",
        "ctrl" | "control" => "ctrl",
        "alt" | "option" | "mod1" => "alt",
        "altgr" => "alt_r",
        "enter" | "return" => "ret",
        "space" => "spc",
        "escape" => "esc",
        "del" => "delete",
        "pageup" => "pgup",
        "pagedown" => "pgdn",
        "plus" => "equal",
        _ => {
            // A single character names its key, e.g. "t", "1", "/".
            let mut chars = lower.chars();
            if let (Some(c), None) = (chars.next(), chars.next())
                && let Some((code, _)) = char_key(c)
            {
                return code.to_string();
            }
            return lower;
        }
    };
    code.to_string()
}

/// The chords that type `text`, one per character.
pub fn text(text: &str) -> Result<Vec<Vec<String>>> {
    text.chars()
        .map(|c| match char_key(c) {
            Some((code, true)) => Ok(vec!["shift".to_string(), code.to_string()]),
            Some((code, false)) => Ok(vec![code.to_string()]),
            None => bail!("can't type {c:?}; only US keyboard characters are supported"),
        })
        .collect()
}

/// The key that types `c` on a US layout, and whether it needs shift.
fn char_key(c: char) -> Option<(&'static str, bool)> {
    const LETTERS: [&str; 26] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v",
        "w", "x", "y", "z",
    ];
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const SHIFTED_DIGITS: &str = ")!@#$%^&*(";
    Some(match c {
        'a'..='z' => (LETTERS[c as usize - 'a' as usize], false),
        'A'..='Z' => (LETTERS[c as usize - 'A' as usize], true),
        '0'..='9' => (DIGITS[c as usize - '0' as usize], false),
        ' ' => ("spc", false),
        '\n' => ("ret", false),
        '\t' => ("tab", false),
        '-' => ("minus", false),
        '_' => ("minus", true),
        '=' => ("equal", false),
        '+' => ("equal", true),
        '[' => ("bracket_left", false),
        '{' => ("bracket_left", true),
        ']' => ("bracket_right", false),
        '}' => ("bracket_right", true),
        ';' => ("semicolon", false),
        ':' => ("semicolon", true),
        '\'' => ("apostrophe", false),
        '"' => ("apostrophe", true),
        '`' => ("grave_accent", false),
        '~' => ("grave_accent", true),
        '\\' => ("backslash", false),
        '|' => ("backslash", true),
        ',' => ("comma", false),
        '<' => ("comma", true),
        '.' => ("dot", false),
        '>' => ("dot", true),
        '/' => ("slash", false),
        '?' => ("slash", true),
        _ => return SHIFTED_DIGITS.find(c).map(|i| (DIGITS[i], true)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        assert_eq!(chord("super+shift+t").unwrap(), ["meta_l", "shift", "t"]);
        assert_eq!(chord("Ctrl-Alt-F2").unwrap(), ["ctrl", "alt", "f2"]);
        assert_eq!(chord("super+Return").unwrap(), ["meta_l", "ret"]);
        assert_eq!(chord("ctrl+/").unwrap(), ["ctrl", "slash"]);
        assert_eq!(chord("ctrl+minus").unwrap(), ["ctrl", "minus"]);
        assert!(chord("ctrl++").is_err());
        assert!(chord("").is_err());
    }

    #[test]
    fn typing() {
        assert_eq!(text("a B").unwrap(), [vec!["a"], vec!["spc"], vec!["shift", "b"]]);
        assert_eq!(text("$_\n").unwrap(), [vec!["shift", "4"], vec!["shift", "minus"], vec!["ret"]]);
        assert!(text("é").is_err());
    }
}
