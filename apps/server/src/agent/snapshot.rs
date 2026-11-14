//! Snapshot attach (decision 154, AM110): a cold open gets the screen at
//! offset S, rendered from the output cache, instead of the whole cache.
//!
//! The rendering is read-only and discardable. It never answers a query,
//! never feeds input and never rewrites `output`; it is VT bytes that rebuild
//! the grid, cursor and the keyboard, mouse and screen modes, and nothing
//! else (no title, no queries).
//!
//! `vt100` restores the grid and its own modes. A [`Layer`] of callbacks keeps
//! the ones it drops: the kitty flags, modifyOtherKeys, focus (1004) and theme
//! reports (2031). Once the cache has evicted bytes, a parse from the floor
//! would miss modes set before it (Claude Code sends `?1049h` once), so the
//! cache feeds what it evicts to a [`ModeTracker`] and the render starts from
//! its state.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::num::NonZeroU16;

use serde::Serialize;

/// A cold attach replays instead when the cache holds no more than this.
pub const MIN_RETAINED: u64 = 64 * 1024;
/// Scrollback rows sent above a main-screen session's screen.
pub const HISTORY_ROWS: usize = 1000;
/// The deepest kitty keyboard stack kept; a deeper push drops the oldest.
const KITTY_STACK: usize = 16;

/// The terminal modes a snapshot restores.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Modes {
    alt: bool,
    app_cursor: bool,
    app_keypad: bool,
    cursor_hidden: bool,
    bracketed_paste: bool,
    /// 0, or the DECSET number: 9, 1000, 1002, 1003.
    mouse: u16,
    /// 0, 1005 or 1006.
    mouse_encoding: u16,
    kitty: u16,
    kitty_stack: Vec<u16>,
    modify_other_keys: u16,
    focus: bool,
    theme: bool,
}

impl Modes {
    /// A CSI sequence. `native` is false inside `vt100`, which handles the
    /// screen's own modes and passes only the rest here.
    fn csi(&mut self, i1: Option<u8>, params: &[u16], c: char, native: bool) {
        let first = params.first().copied();
        match (i1, c) {
            (Some(b'?'), 'h' | 'l') => {
                let on = c == 'h';
                for &mode in params {
                    match mode {
                        1004 => self.focus = on,
                        2031 => self.theme = on,
                        _ if native => self.decset(mode, on),
                        _ => {}
                    }
                }
            }
            (Some(b'>'), 'u') => {
                if self.kitty_stack.len() == KITTY_STACK {
                    self.kitty_stack.remove(0);
                }
                self.kitty_stack.push(self.kitty);
                self.kitty = first.unwrap_or(0) & 0x1f;
            }
            (Some(b'<'), 'u') => {
                for _ in 0..first.unwrap_or(1).max(1) {
                    self.kitty = self.kitty_stack.pop().unwrap_or(0);
                }
            }
            (Some(b'='), 'u') => {
                let flags = first.unwrap_or(0) & 0x1f;
                match params.get(1).copied().unwrap_or(1) {
                    2 => self.kitty |= flags,
                    3 => self.kitty &= !flags,
                    _ => self.kitty = flags,
                }
            }
            (Some(b'>'), 'm') if first == Some(4) => {
                self.modify_other_keys = params.get(1).copied().unwrap_or(0);
            }
            (Some(b'>'), 'n') if first == Some(4) => self.modify_other_keys = 0,
            _ => {}
        }
    }

    /// The modes `vt100` also tracks, as it treats them.
    fn decset(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.app_cursor = on,
            25 => self.cursor_hidden = !on,
            47 | 1047 | 1049 => self.alt = on,
            2004 => self.bracketed_paste = on,
            9 | 1000 | 1002 | 1003 if on => self.mouse = mode,
            9 | 1000 | 1002 | 1003 if self.mouse == mode => self.mouse = 0,
            1005 | 1006 if on => self.mouse_encoding = mode,
            1005 | 1006 if self.mouse_encoding == mode => self.mouse_encoding = 0,
            _ => {}
        }
    }

    /// The bytes that put a fresh `vt100` into the modes it tracks itself.
    fn native_prefix(&self) -> String {
        let mut out = String::new();
        if self.alt {
            out.push_str("\x1b[?1049h");
        }
        if self.app_cursor {
            out.push_str("\x1b[?1h");
        }
        if self.app_keypad {
            out.push_str("\x1b=");
        }
        if self.cursor_hidden {
            out.push_str("\x1b[?25l");
        }
        if self.bracketed_paste {
            out.push_str("\x1b[?2004h");
        }
        for mode in [self.mouse, self.mouse_encoding] {
            if mode != 0 {
                let _ = write!(out, "\x1b[?{mode}h");
            }
        }
        out
    }
}

/// The modes in force at the cache's floor, fed the bytes the cache evicts
/// as it evicts them. No grid: it costs a `vte` parse per evicted byte.
#[derive(Default)]
pub struct ModeTracker {
    parser: vte::Parser,
    modes: Modes,
}

impl std::fmt::Debug for ModeTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModeTracker")
            .field("modes", &self.modes)
            .finish_non_exhaustive()
    }
}

impl ModeTracker {
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.modes, bytes);
    }

    pub fn modes(&self) -> &Modes {
        &self.modes
    }
}

impl vte::Perform for Modes {
    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], _: bool, c: char) {
        // Every mode tracked here has an intermediate; cursor moves and SGR,
        // most of the stream, have none.
        if let Some(&i1) = intermediates.first() {
            let params: Vec<u16> = params.iter().map(|p| p[0]).collect();
            self.csi(Some(i1), &params, c, true);
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _: bool, byte: u8) {
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            b'=' => self.app_keypad = true,
            b'>' => self.app_keypad = false,
            b'c' => *self = Modes::default(),
            _ => {}
        }
    }
}

/// The render's callbacks: the modes `vt100` drops, and the main screen's
/// rows scrolled off the top, counted or, with `capture`, kept.
struct Layer {
    modes: Modes,
    scrolled: usize,
    capture: bool,
    history: VecDeque<String>,
}

impl Layer {
    fn new(modes: Modes, capture: bool) -> Self {
        Self {
            modes,
            scrolled: 0,
            capture,
            history: VecDeque::new(),
        }
    }
}

impl vt100::Callbacks for Layer {
    fn unhandled_csi(
        &mut self,
        _: &mut vt100::Screen,
        i1: Option<u8>,
        _: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let params: Vec<u16> = params.iter().map(|p| p[0]).collect();
        self.modes.csi(i1, &params, c, false);
    }

    fn on_scroll(&mut self, row: vt100::capture::RowContents<'_>, alternate_screen: bool) {
        if alternate_screen {
            return;
        }
        self.scrolled += 1;
        if !self.capture {
            return;
        }
        let mut line = if self.history.len() == HISTORY_ROWS {
            let mut reused = self.history.pop_front().unwrap_or_default();
            reused.clear();
            reused
        } else {
            String::new()
        };
        let _ = row.write_formatted_basic(&mut line, &mut Default::default());
        self.history.push_back(line);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModesView {
    pub kitty_flags: u16,
    pub modify_other_keys: u16,
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub alt_screen: bool,
    pub mouse: &'static str,
    pub mouse_encoding: &'static str,
    pub focus_reporting: bool,
    pub theme_reports: bool,
    pub cursor_visible: bool,
}

#[derive(Debug)]
pub struct Rendered {
    pub data: Vec<u8>,
    pub alt: bool,
    pub modes: ModesView,
    pub history_rows: usize,
}

/// Sequences are short: with no `ESC` or `\n` this close to the floor, the
/// floor is in text, and skipping further would drop it.
const ALIGN_WITHIN: usize = 4096;

/// Where a parse from a floor above 0 starts: the first `ESC` or `\n`, so it
/// never begins inside a sequence, or in plain text past any partial UTF-8
/// character.
fn align(bytes: &[u8]) -> usize {
    let near = &bytes[..bytes.len().min(ALIGN_WITHIN)];
    near.iter()
        .position(|&b| b == 0x1b || b == b'\n')
        .unwrap_or_else(|| {
            near.iter()
                .take_while(|&&b| (0x80..0xc0).contains(&b))
                .count()
        })
}

/// How far back the history pass may start; 1,000 rows rarely need more.
const HISTORY_BYTES: usize = 512 * 1024;

/// Renders `bytes`, the cache from `floor`, at the PTY's size.
///
/// From a floor above 0 the parse starts [aligned](align), from the
/// tracker's `seed` modes. Formatting every row that scrolls would double the
/// cost of a 4 MiB main-screen parse, so the history comes from a second
/// parse of only the tail that holds its last rows.
pub fn render(bytes: &[u8], floor: u64, seed: &Modes, (cols, rows): (u16, u16)) -> Rendered {
    let size = |n: u16, max: u16| NonZeroU16::new(n.clamp(1, max)).expect("clamped above 0");
    let (cols, rows) = (size(cols, 1000), size(rows, 500));
    let (start, seed) = if floor == 0 {
        (0, Modes::default())
    } else {
        (align(bytes), seed.clone())
    };
    let mut parser =
        vt100::Parser::new_with_callbacks(rows, cols, 0, Layer::new(seed.clone(), false));
    parser.process(seed.native_prefix().as_bytes());
    parser.process(&bytes[start..]);

    let screen = parser.screen();
    let layer = parser.callbacks();
    let alt = screen.alternate_screen();
    let history = if alt || layer.scrolled == 0 {
        VecDeque::new()
    } else {
        let mut rows = history(&bytes[start..], (cols, rows));
        rows.drain(..rows.len().saturating_sub(layer.scrolled));
        rows
    };
    let mut out = String::from("\x1bc");
    let history_rows = history.len();
    if history_rows > 0 {
        for line in &history {
            out.push_str(line);
            out.push_str("\x1b[m\r\n");
        }
        // Scroll the last history row off the screen the grid redraws.
        for _ in 1..rows.get() {
            out.push('\n');
        }
    }
    if alt {
        out.push_str("\x1b[?1049h");
    }
    out.push_str(&screen.state_formatted());
    let m = &layer.modes;
    if m.kitty != 0 {
        let _ = write!(out, "\x1b[={};1u", m.kitty);
    }
    if m.modify_other_keys != 0 {
        let _ = write!(out, "\x1b[>4;{}m", m.modify_other_keys);
    }
    if m.focus {
        out.push_str("\x1b[?1004h");
    }
    if m.theme {
        out.push_str("\x1b[?2031h");
    }
    Rendered {
        data: out.into_bytes(),
        alt,
        modes: view(screen, m),
        history_rows,
    }
}

/// The main screen's last [`HISTORY_ROWS`] scrolled rows, from a parse of
/// the shortest tail with enough lines, starting at a line, and at most
/// [`HISTORY_BYTES`].
fn history(bytes: &[u8], (cols, rows): (NonZeroU16, NonZeroU16)) -> VecDeque<String> {
    let lines = HISTORY_ROWS + usize::from(rows.get()) + 1;
    let window = bytes.len().saturating_sub(HISTORY_BYTES);
    let newlines = bytes[window..]
        .iter()
        .enumerate()
        .rev()
        .filter(|&(_, &b)| b == b'\n')
        .map(|(i, _)| window + i + 1);
    let from = match newlines.clone().nth(lines - 1) {
        Some(at) => at,
        None if window == 0 => 0,
        None => newlines.clone().next_back().unwrap_or_else(|| {
            window + bytes[window..].iter().position(|&b| b == 0x1b).unwrap_or(0)
        }),
    };
    let mut parser =
        vt100::Parser::new_with_callbacks(rows, cols, 0, Layer::new(Modes::default(), true));
    parser.process(&bytes[from..]);
    std::mem::take(&mut parser.callbacks_mut().history)
}

fn view(screen: &vt100::Screen, m: &Modes) -> ModesView {
    use vt100::{MouseProtocolEncoding as E, MouseProtocolMode as P};
    ModesView {
        kitty_flags: m.kitty,
        modify_other_keys: m.modify_other_keys,
        bracketed_paste: screen.bracketed_paste(),
        app_cursor: screen.application_cursor(),
        app_keypad: screen.application_keypad(),
        alt_screen: screen.alternate_screen(),
        mouse: match screen.mouse_protocol_mode() {
            P::None => "none",
            P::Press => "press",
            P::PressRelease => "press_release",
            P::ButtonMotion => "button_motion",
            P::AnyMotion => "any_motion",
        },
        mouse_encoding: match screen.mouse_protocol_encoding() {
            E::Default => "default",
            E::Utf8 => "utf8",
            E::Sgr => "sgr",
        },
        focus_reporting: m.focus,
        theme_reports: m.theme,
        cursor_visible: !screen.hide_cursor(),
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
