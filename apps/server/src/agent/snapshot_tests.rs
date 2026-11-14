use super::*;
use crate::agent::cache::OutputCache;

const CAPTURE: &[u8] = include_bytes!("claude_code_screen.bin");
const MIB4: usize = 4 * 1024 * 1024;

fn replace(hay: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// The Claude Code capture repeated to `len`, with varied answers, and the
/// modes it sets once (`?1049h`, 1004, 2031, `2J`) only in the first copy.
fn claude_code_corpus(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + CAPTURE.len());
    let mut n = 0u32;
    while out.len() < len {
        let word = format!("w{n:08x}");
        let mut copy = replace(CAPTURE, b"first-answer-words", word.as_bytes());
        copy = replace(
            &copy,
            b"second-answer-words",
            format!("{word}-2").as_bytes(),
        );
        if n > 0 {
            for once in [
                &b"\x1b[?1049h"[..],
                b"\x1b[?2031h",
                b"\x1b[?1004h",
                b"\x1b[2J",
            ] {
                copy = replace(&copy, once, b"");
            }
        }
        out.extend_from_slice(&copy);
        n += 1;
    }
    out.truncate(len);
    out
}

/// Main-screen output, as a PTY sends `git log -p --color`: coloured lines,
/// some longer than the screen, `\r\n` endings.
fn main_screen_corpus(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 512);
    let mut n = 0u64;
    while out.len() < len {
        let colour = 31 + n % 6;
        let width = 20 + (n * 37 % 180) as usize;
        let text: String = (0..width)
            .map(|i| char::from(b'a' + ((n as usize + i) % 26) as u8))
            .collect();
        out.extend_from_slice(format!("\x1b[{colour}m{n:06}\x1b[m {text}\r\n").as_bytes());
        n += 1;
    }
    out.truncate(len);
    out
}

fn parser(size: (u16, u16)) -> vt100::Parser<Layer> {
    let nz = |n| NonZeroU16::new(n).unwrap();
    let layer = Layer::new(Modes::default(), true);
    vt100::Parser::new_with_callbacks(nz(size.1), nz(size.0), 0, layer)
}

fn parsed(bytes: &[u8], size: (u16, u16)) -> vt100::Parser<Layer> {
    let mut p = parser(size);
    p.process(bytes);
    p
}

/// What a client's terminal shows, and every mode a snapshot restores.
fn assert_same_terminal(want: &vt100::Parser<Layer>, got: &vt100::Parser<Layer>) {
    let (a, b) = (want.screen(), got.screen());
    assert!(
        a.contents_formatted() == b.contents_formatted(),
        "screens differ"
    );
    assert_eq!(a.contents(), b.contents());
    assert_eq!(a.alternate_screen(), b.alternate_screen(), "alt");
    assert_eq!(a.cursor_position(), b.cursor_position(), "cursor");
    assert_eq!(a.input_mode_formatted(), b.input_mode_formatted());
    assert_eq!(
        view(a, &want.callbacks().modes),
        view(b, &got.callbacks().modes)
    );
}

/// Every CSI, OSC and DCS in `data`; a snapshot holds no queries and no title.
#[derive(Default)]
struct Sequences {
    csi: Vec<(Vec<u8>, Vec<u16>, char)>,
    osc: usize,
    dcs: usize,
}

impl vte::Perform for Sequences {
    fn csi_dispatch(&mut self, params: &vte::Params, i: &[u8], _: bool, c: char) {
        self.csi
            .push((i.to_vec(), params.iter().map(|p| p[0]).collect(), c));
    }
    fn osc_dispatch(&mut self, _: &[&[u8]], _: bool) {
        self.osc += 1;
    }
    fn hook(&mut self, _: &vte::Params, _: &[u8], _: bool, _: char) {
        self.dcs += 1;
    }
}

fn assert_no_queries(data: &[u8]) {
    let mut seen = Sequences::default();
    vte::Parser::new().advance(&mut seen, data);
    assert_eq!((seen.osc, seen.dcs), (0, 0), "no OSC (title) and no DCS");
    for (i, params, c) in &seen.csi {
        let query = match (i.as_slice(), c) {
            (_, 'c') => true,                         // DA1, DA2
            (_, 'n') => true,                         // DSR, CPR
            (b">", 'q') => true,                      // XTVERSION
            (b"?", 'u') => true,                      // kitty flags query
            (b"?" | b"?$" | b"$", 'p') => true,       // DECRQM
            (b"", 't') => params.first() != Some(&8), // XTWINOPS reports
            _ => false,
        };
        assert!(!query, "a query in the snapshot: {i:?} {params:?} {c}");
    }
}

#[test]
fn the_claude_code_capture_round_trips_through_a_fresh_terminal() {
    let size = (100, 30);
    let r = render(CAPTURE, 0, &Modes::default(), size);
    let want = parsed(CAPTURE, size);
    assert_same_terminal(&want, &parsed(&r.data, size));
    assert!(r.alt);
    assert_eq!(
        r.history_rows, 0,
        "an alt-screen session sends its screen only"
    );
    assert_eq!(r.modes.kitty_flags, 5);
    assert_eq!(r.modes.modify_other_keys, 2);
    assert!(r.modes.focus_reporting && r.modes.theme_reports && r.modes.bracketed_paste);
    assert_eq!(
        (r.modes.mouse, r.modes.mouse_encoding),
        ("any_motion", "sgr")
    );
    assert!(
        r.data.starts_with(b"\x1bc\x1b[?1049h"),
        "reset, then the alt screen"
    );
    assert_no_queries(&r.data);
    let text = String::from_utf8_lossy(&r.data);
    assert!(text.contains("second-answer-words"), "{text}");
}

#[test]
fn a_4_mib_alt_screen_session_renders_to_a_few_kib_and_round_trips() {
    let size = (120, 40);
    let bytes = claude_code_corpus(MIB4);
    let started = std::time::Instant::now();
    let r = render(&bytes, 0, &Modes::default(), size);
    let took = started.elapsed();
    assert_same_terminal(&parsed(&bytes, size), &parsed(&r.data, size));
    assert!(r.alt);
    assert!(r.data.len() < 8 * 1024, "{} bytes", r.data.len());
    assert_no_queries(&r.data);
    eprintln!("4 MiB alt-screen: {} B in {took:?}", r.data.len());
    // A-154-9, measured in a release build.
    if !cfg!(debug_assertions) {
        assert!(took.as_millis() <= 60, "{took:?}");
    }
}

#[test]
fn a_main_screen_session_gets_its_last_1000_rows_above_the_screen() {
    let size = (120, 40);
    let bytes = main_screen_corpus(MIB4);
    let started = std::time::Instant::now();
    let r = render(&bytes, 0, &Modes::default(), size);
    let took = started.elapsed();
    assert!(!r.alt);
    assert_eq!(r.history_rows, HISTORY_ROWS);
    assert_no_queries(&r.data);
    // vt100's own parse of 4 MiB of scrolling lines is ~55 ms here; the
    // history pass adds ~4 ms. Not asserted: A-154-9 is the alt screen's.
    eprintln!("4 MiB main-screen: {} B in {took:?}", r.data.len());

    let (want, got) = (parsed(&bytes, size), parsed(&r.data, size));
    assert_same_terminal(&want, &got);
    let plain = |history: &VecDeque<String>| -> Vec<String> {
        history
            .iter()
            .map(|l| {
                let mut p = parsed(l.as_bytes(), (size.0, 1));
                p.process(b"\r");
                p.screen().contents()
            })
            .collect()
    };
    let (sent, kept) = (
        plain(&want.callbacks().history),
        plain(&got.callbacks().history),
    );
    assert_eq!(
        kept.len(),
        HISTORY_ROWS,
        "the history scrolled off, the screen did not"
    );
    assert_eq!(kept, sent);
}

#[test]
fn queries_in_the_output_never_reach_the_snapshot() {
    let mut bytes = b"\x1b]2;a title\x07\x1b[c\x1b[>0q\x1b[?u\x1b[6n\x1b[?2026$p".to_vec();
    bytes.extend(claude_code_corpus(256 * 1024));
    bytes.extend_from_slice(b"\x1b[5n\x1b[18t\x1bP+q4d73\x1b\\");
    let r = render(&bytes, 0, &Modes::default(), (100, 30));
    assert_no_queries(&r.data);
    assert!(!String::from_utf8_lossy(&r.data).contains("a title"));
}

#[test]
fn past_an_evicted_1049h_the_tracker_keeps_the_alt_screen() {
    // The research's 50/50: a parse from the floor misses `?1049h` every
    // time. The tracker, fed what the cache evicted, does not.
    let size = (120, 40);
    let bytes = claude_code_corpus(MIB4);
    let whole = parsed(&bytes, size);
    for n in 0..50u64 {
        let capacity = 256 * 1024 + n as usize * 37;
        let mut cache = OutputCache::new(capacity);
        for (i, piece) in bytes.chunks(64 * 1024).enumerate() {
            cache.insert((i * 64 * 1024) as u64, piece);
        }
        assert!(cache.floor() > 0);
        let r = render(&cache.retained(), cache.floor(), cache.floor_modes(), size);
        assert!(r.alt, "floor {}: lost the alt screen", cache.floor());
        let got = parsed(&r.data, size);
        assert_eq!(
            view(got.screen(), &got.callbacks().modes),
            view(whole.screen(), &whole.callbacks().modes),
            "floor {}",
            cache.floor()
        );
        assert_no_queries(&r.data);
        if n == 0 {
            let untracked = render(&cache.retained(), cache.floor(), &Modes::default(), size);
            assert!(!untracked.alt, "without the tracker the alt screen is lost");
        }
    }
}

#[test]
fn a_floor_inside_a_csi_prints_no_bracket() {
    // The floor fell inside `ESC [30;1H`: its tail must not print.
    let bytes = b"30;1Hgarbage\x1b[1;1Hclean\r\nline two";
    let r = render(bytes, 1000, &Modes::default(), (40, 5));
    let screen = parsed(&r.data, (40, 5)).screen().contents();
    assert!(!screen.contains('['), "{screen:?}");
    assert!(!screen.contains("30;1H"), "{screen:?}");
    assert!(screen.starts_with("clean\nline two"), "{screen:?}");

    // And on the real stream, from floors inside its sequences.
    let corpus = claude_code_corpus(512 * 1024);
    let tail = corpus.len() - 128 * 1024;
    let inside: Vec<usize> = (tail..tail + 8192)
        .filter(|&i| corpus[i - 1] == 0x1b && corpus[i] == b'[')
        .map(|i| i + 1)
        .take(20)
        .collect();
    assert_eq!(inside.len(), 20);
    for at in inside {
        let end = at
            + corpus[at..]
                .iter()
                .position(|b| (0x40..=0x7e).contains(b))
                .unwrap();
        let params = String::from_utf8_lossy(&corpus[at..=end]).into_owned();
        let r = render(&corpus[at..], at as u64, &Modes::default(), (100, 30));
        let screen = parsed(&r.data, (100, 30)).screen().contents();
        assert!(
            !screen.starts_with(&params),
            "floor {at}: {params:?} printed"
        );
    }
}

#[test]
fn the_tracker_follows_the_kitty_stack_and_the_other_modes() {
    let mut t = ModeTracker::default();
    t.feed(b"\x1b[>5u\x1b[>1u");
    assert_eq!(
        (t.modes().kitty, t.modes().kitty_stack.clone()),
        (1, vec![0, 5])
    );
    t.feed(b"\x1b[<u");
    assert_eq!(t.modes().kitty, 5);
    t.feed(b"\x1b[=2;2u");
    assert_eq!(t.modes().kitty, 7);
    t.feed(b"\x1b[=1;3u");
    assert_eq!(t.modes().kitty, 6);
    t.feed(b"\x1b[<5u");
    assert_eq!(t.modes().kitty, 0, "popping past the bottom gives 0");
    for _ in 0..40 {
        t.feed(b"\x1b[>1u");
    }
    assert_eq!(t.modes().kitty_stack.len(), KITTY_STACK);

    // A sequence split across two evictions still lands.
    t.feed(b"\x1b[>4;");
    t.feed(b"2m\x1b[?1004;20");
    t.feed(b"04h\x1b[?1047h\x1b[?1003h\x1b[?1006h\x1b=");
    let m = t.modes();
    assert_eq!(m.modify_other_keys, 2);
    assert!(m.focus && m.bracketed_paste && m.alt && m.app_keypad);
    assert_eq!((m.mouse, m.mouse_encoding), (1003, 1006));
    t.feed(b"\x1b[?1000l");
    assert_eq!(
        t.modes().mouse,
        1003,
        "resetting another mouse mode keeps this one"
    );
    t.feed(b"\x1b[?1003l\x1b[?1047l\x1b[>4n");
    assert_eq!(
        (t.modes().mouse, t.modes().alt, t.modes().modify_other_keys),
        (0, false, 0)
    );
    t.feed(b"\x1b[?2031h\x1b[?25l\x1bc");
    assert_eq!(t.modes(), &Modes::default(), "RIS resets everything");
}

#[test]
fn the_seed_restores_every_tracked_mode_into_a_fresh_render() {
    let mut t = ModeTracker::default();
    t.feed(b"\x1b[?1049h\x1b[?1h\x1b=\x1b[?25l\x1b[?2004h\x1b[?1002h\x1b[?1006h");
    t.feed(b"\x1b[>5u\x1b[>4;1m\x1b[?1004h\x1b[?2031h");
    let r = render(b"\nhello", 4096, t.modes(), (40, 10));
    assert!(r.alt);
    assert_eq!(
        r.modes,
        ModesView {
            kitty_flags: 5,
            modify_other_keys: 1,
            bracketed_paste: true,
            app_cursor: true,
            app_keypad: true,
            alt_screen: true,
            mouse: "button_motion",
            mouse_encoding: "sgr",
            focus_reporting: true,
            theme_reports: true,
            cursor_visible: false,
        }
    );
    let got = parsed(&r.data, (40, 10));
    assert_eq!(view(got.screen(), &got.callbacks().modes), r.modes);
    assert!(got.screen().contents().contains("hello"));
}

#[test]
fn the_cache_feeds_exactly_what_it_evicts_to_the_tracker() {
    let mut c = OutputCache::new(8);
    c.insert(0, b"\x1b[?1049h");
    assert!(!c.floor_modes().alt, "nothing evicted yet");
    c.insert(8, b"abcdefgh");
    assert_eq!(c.floor(), 8);
    assert!(c.floor_modes().alt);
    c.insert(16, b"\x1b[?1049l--------");
    assert_eq!(c.floor(), 24);
    assert!(
        !c.floor_modes().alt,
        "a whole post past capacity is fed too"
    );
    assert_eq!(c.retained(), b"--------");

    // A jump forward keeps the modes at the old end as its guess.
    let mut j = OutputCache::new(64);
    j.insert(0, b"\x1b[>5u");
    j.insert(1000, b"later");
    assert_eq!((j.floor(), j.floor_modes().kitty), (1000, 5));
}

/// A-154-9's timing, in release: `cargo test --release -- --ignored snapshot_timing`.
#[test]
#[ignore]
fn snapshot_timing() {
    for (name, bytes) in [
        ("alt", claude_code_corpus(MIB4)),
        ("main", main_screen_corpus(MIB4)),
    ] {
        let mut times: Vec<_> = (0..7)
            .map(|_| {
                let t = std::time::Instant::now();
                let r = render(&bytes, 0, &Modes::default(), (120, 40));
                (t.elapsed(), r.data.len())
            })
            .collect();
        times.sort();
        eprintln!(
            "{name}: median {:?} (min {:?}), {} B",
            times[3].0, times[0].0, times[3].1
        );
    }
    let bytes = claude_code_corpus(8 * 1024 * 1024);
    let mut c = OutputCache::new(crate::agent::cache::DEFAULT_CAPACITY);
    let t = std::time::Instant::now();
    for (i, piece) in bytes.chunks(64 * 1024).enumerate() {
        c.insert((i * 64 * 1024) as u64, piece);
    }
    eprintln!("tracker: 8 MiB in, 4 MiB evicted, {:?}", t.elapsed());
}

#[test]
fn lines_that_only_wrap_still_leave_history() {
    let bytes = vec![b'x'; 1024 * 1024];
    let r = render(&bytes, 0, &Modes::default(), (80, 24));
    assert!(!r.alt);
    assert_eq!(r.history_rows, HISTORY_ROWS);
    assert_same_terminal(&parsed(&bytes, (80, 24)), &parsed(&r.data, (80, 24)));
}

#[test]
fn a_floor_in_plain_text_starts_there_past_a_split_character() {
    let mut bytes = "é".as_bytes()[1..].to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 64 * 1024));
    let r = render(&bytes, 9999, &Modes::default(), (80, 24));
    let screen = parsed(&r.data, (80, 24)).screen().contents();
    assert!(
        screen.starts_with("xxxx") && !screen.contains('\u{fffd}'),
        "{screen:?}"
    );
    assert_eq!(r.history_rows, 64 * 1024 / 80 - 23);
}
