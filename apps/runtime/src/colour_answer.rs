//! The brightness a session's client answered its program's OSC 10/11
//! colour query with (decision 157, 157g), read from the input stream the
//! host writes to the PTY. Input is only observed, never rewritten.

use serde::Serialize;

/// Longer than any colour reply; anything longer is not one.
const MAX_BODY: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Brightness {
    Light,
    Dark,
}

impl Brightness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn opposite(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    Body,
    BodyEscape,
}

/// Bounded parse state for one session's input, across writes.
#[derive(Debug, Default)]
pub struct ColourAnswers {
    state: State,
    body: Vec<u8>,
    background: Option<Brightness>,
    foreground: Option<Brightness>,
}

impl ColourAnswers {
    /// What the answers so far say: the background's, else the opposite
    /// of the foreground's.
    pub fn current(&self) -> Option<Brightness> {
        self.background
            .or(self.foreground.map(Brightness::opposite))
    }

    /// Returns the new brightness when this input changed it.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<Brightness> {
        let before = self.current();
        for &b in bytes {
            self.step(b);
        }
        let now = self.current();
        (now != before).then_some(now).flatten()
    }

    fn step(&mut self, b: u8) {
        self.state = match (self.state, b) {
            (State::Ground, 0x1b) => State::Escape,
            (State::Ground, _) => State::Ground,
            (State::Escape | State::BodyEscape, b']') => {
                self.body.clear();
                State::Body
            }
            (State::BodyEscape, b'\\') => {
                self.finish();
                State::Ground
            }
            (State::Escape | State::BodyEscape, 0x1b) => State::Escape,
            (State::Escape | State::BodyEscape, _) => State::Ground,
            (State::Body, 0x07) => {
                self.finish();
                State::Ground
            }
            (State::Body, 0x1b) => State::BodyEscape,
            (State::Body, b) if (0x20..0x7f).contains(&b) && self.body.len() < MAX_BODY => {
                self.body.push(b);
                State::Body
            }
            (State::Body, _) => State::Ground,
        };
    }

    fn finish(&mut self) {
        let body = std::mem::take(&mut self.body);
        let Some((code, spec)) = body.split_first_chunk::<2>() else {
            return;
        };
        let Some(brightness) = spec.strip_prefix(b";").and_then(brightness_of) else {
            return;
        };
        match code {
            b"11" => self.background = Some(brightness),
            b"10" => self.foreground = Some(brightness),
            _ => {}
        }
    }
}

/// `rgb:R/G/B` with 1–4 hex digits a channel, or `#rgb`, `#rrggbb`,
/// `#rrrgggbbb`, `#rrrrggggbbbb`: light when its relative luminance is over
/// one half.
fn brightness_of(spec: &[u8]) -> Option<Brightness> {
    let spec = std::str::from_utf8(spec).ok()?;
    let channels: Vec<f64> = if let Some(rgb) = spec.strip_prefix("rgb:") {
        rgb.split('/').map(channel).collect::<Option<_>>()?
    } else {
        let hex = spec.strip_prefix('#')?;
        let n = hex.len() / 3;
        if hex.len() % 3 != 0 || !(1..=4).contains(&n) {
            return None;
        }
        (0..3)
            .map(|i| channel(&hex[i * n..(i + 1) * n]))
            .collect::<Option<_>>()?
    };
    let [r, g, b] = channels[..] else {
        return None;
    };
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    Some(if luminance > 0.5 {
        Brightness::Light
    } else {
        Brightness::Dark
    })
}

fn channel(hex: &str) -> Option<f64> {
    if !(1..=4).contains(&hex.len()) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let max = (1u32 << (4 * hex.len())) - 1;
    Some(f64::from(u32::from_str_radix(hex, 16).ok()?) / f64::from(max))
}

fn linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(chunks: &[&[u8]]) -> Option<Brightness> {
        let mut a = ColourAnswers::default();
        for c in chunks {
            a.feed(c);
        }
        a.current()
    }

    #[test]
    fn a_background_reply_with_either_terminator() {
        assert_eq!(
            fed(&[b"\x1b]11;rgb:ffff/ffff/ffff\x07"]),
            Some(Brightness::Light)
        );
        assert_eq!(
            fed(&[b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\"]),
            Some(Brightness::Dark)
        );
    }

    #[test]
    fn a_reply_split_across_writes() {
        let reply = b"\x1b]11;rgb:fafa/f9f9/f5f5\x1b\\";
        for cut in 1..reply.len() {
            let (a, b) = reply.split_at(cut);
            assert_eq!(fed(&[a, b]), Some(Brightness::Light), "cut at {cut}");
        }
        assert_eq!(
            fed(&[b"\x1b", b"]", b"1", b"1;rgb:00/00/00", b"\x1b", b"\\"]),
            Some(Brightness::Dark)
        );
    }

    #[test]
    fn two_and_four_digit_channels_and_hash_forms() {
        assert_eq!(fed(&[b"\x1b]11;rgb:ff/ff/ff\x07"]), Some(Brightness::Light));
        assert_eq!(fed(&[b"\x1b]11;rgb:20/20/20\x07"]), Some(Brightness::Dark));
        assert_eq!(fed(&[b"\x1b]11;#ffffff\x07"]), Some(Brightness::Light));
        assert_eq!(fed(&[b"\x1b]11;#101418\x07"]), Some(Brightness::Dark));
    }

    #[test]
    fn the_threshold_is_half_relative_luminance() {
        // #bcbcbc is 0.503, #bababa 0.491.
        assert_eq!(fed(&[b"\x1b]11;#bcbcbc\x07"]), Some(Brightness::Light));
        assert_eq!(fed(&[b"\x1b]11;#bababa\x07"]), Some(Brightness::Dark));
    }

    #[test]
    fn a_foreground_alone_implies_the_opposite_and_a_background_wins() {
        assert_eq!(
            fed(&[b"\x1b]10;rgb:0000/0000/0000\x07"]),
            Some(Brightness::Light)
        );
        assert_eq!(
            fed(&[
                b"\x1b]11;rgb:0000/0000/0000\x07",
                b"\x1b]10;rgb:0000/0000/0000\x07"
            ]),
            Some(Brightness::Dark)
        );
    }

    #[test]
    fn feed_says_only_a_change() {
        let mut a = ColourAnswers::default();
        assert_eq!(a.feed(b"\x1b]11;#ffffff\x07"), Some(Brightness::Light));
        assert_eq!(a.feed(b"\x1b]11;#fefefe\x07"), None);
        assert_eq!(a.feed(b"\x1b]11;#000000\x07"), Some(Brightness::Dark));
    }

    #[test]
    fn typed_text_and_other_sequences_are_not_replies() {
        assert_eq!(
            fed(&[
                b"echo ]11;rgb:ffff/ffff/ffff then 11;#ffffff\r",
                b"\x1b[A\x1b[B\x1bOP\x1b]12;#ffffff\x07",
                b"\x1b]4;1;rgb:ffff/ffff/ffff\x07",
                b"\x1b]11;?\x07\x1b]11;rgb:zz/zz/zz\x07",
                b"\x1b]11;rgb:ffff/ffff/ffff\r\x07",
            ]),
            None
        );
    }

    #[test]
    fn an_overlong_body_is_dropped_and_parsing_recovers() {
        let mut long = b"\x1b]11;".to_vec();
        long.extend(std::iter::repeat_n(b'f', 10_000));
        long.push(0x07);
        let mut a = ColourAnswers::default();
        a.feed(&long);
        assert!(a.body.capacity() <= 64);
        assert_eq!(a.current(), None);
        assert_eq!(a.feed(b"\x1b]11;#ffffff\x07"), Some(Brightness::Light));
    }
}
