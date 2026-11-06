//! Copying through OSC 52: the terminal itself puts the text on the system
//! clipboard, so copying works over SSH with no clipboard tool installed.
//!
//! Inside tmux the sequence reaches the outer terminal only with
//! `set -s set-clipboard on`. tmux's default, `external` (still the default in
//! tmux 3.8), sets the clipboard from copy mode but ignores applications, so
//! [`tmux_problem`] checks the setting instead of claiming a copy that never
//! lands.

use std::io::{self, Write};
use std::process::{Command, Stdio};

/// Writes the OSC 52 sequence that copies `text` to `out` and flushes it.
pub fn copy(out: &mut impl Write, text: &str) -> io::Result<()> {
    out.write_all(osc52(text).as_bytes())?;
    out.flush()
}

/// Inside tmux, why a copy will not reach the clipboard, if it will not.
/// Outside tmux, or when tmux cannot be asked, there is nothing to report.
pub fn tmux_problem() -> Option<String> {
    std::env::var_os("TMUX")?;
    let output = Command::new("tmux")
        .args(["show-options", "-sv", "set-clipboard"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    tmux_refusal(&String::from_utf8_lossy(&output.stdout))
}

/// What to tell the viewer when tmux's `set-clipboard` is `set_clipboard`.
fn tmux_refusal(set_clipboard: &str) -> Option<String> {
    match set_clipboard.trim() {
        "on" | "" => None,
        value => Some(format!(
            "tmux ignores the copy while set-clipboard is {value}: \
             add `set -s set-clipboard on` to tmux.conf"
        )),
    }
}

/// `ESC ] 52 ; c ; <base64 of text> BEL`: set the clipboard selection.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, as RFC 4648 section 4.
fn base64(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().enumerate().fold(0u32, |group, (i, &byte)| {
            group | u32::from(byte) << (16 - 8 * i)
        });
        for i in 0..4 {
            if i <= chunk.len() {
                let index = (group >> (18 - 6 * i)) & 0x3f;
                encoded.push(char::from(ALPHABET[index as usize]));
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_4648_test_vectors() {
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (text, encoded) in vectors {
            assert_eq!(base64(text.as_bytes()), encoded, "{text:?}");
        }
    }

    #[test]
    fn base64_uses_the_full_alphabet_for_high_bytes() {
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
    }

    #[test]
    fn osc52_wraps_the_base64_of_the_utf8_text() {
        assert_eq!(osc52("DES-34"), "\x1b]52;c;REVTLTM0\x07");
        assert_eq!(osc52("ñ"), "\x1b]52;c;w7E=\x07");
    }

    #[test]
    fn copy_writes_the_sequence_to_the_terminal() {
        let mut out = Vec::new();

        copy(&mut out, "foo").unwrap();

        assert_eq!(out, b"\x1b]52;c;Zm9v\x07");
    }

    #[test]
    fn tmux_with_set_clipboard_on_passes_the_copy_on() {
        assert_eq!(tmux_refusal("on\n"), None);
    }

    #[test]
    fn tmux_with_any_other_set_clipboard_explains_how_to_fix_it() {
        assert_eq!(
            tmux_refusal("external\n").as_deref(),
            Some(
                "tmux ignores the copy while set-clipboard is external: \
                 add `set -s set-clipboard on` to tmux.conf"
            )
        );
        assert!(tmux_refusal("off").is_some());
    }

    #[test]
    fn an_unreadable_tmux_setting_is_not_reported() {
        assert_eq!(tmux_refusal(""), None);
    }
}
