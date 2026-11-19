import 'dart:convert';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';
import 'package:xterm2/xterm.dart';

import '../keyboard/storm_activators.dart' show stormUsesMetaModifier;
import '../keyboard/storm_commands.dart';
import '../state/terminal_prefs.dart';
import '../ui/clipboard_copy.dart';
import '../ui/controls.dart';
import '../ui/surfaces.dart';
import '../ui/themes/theme_library.dart';
import '../ui/tokens.dart';

/// **The terminal-surface boundary** (freeze §10, gate Q5; decision 77d).
///
/// This is the only file that imports `xterm2`. Everything else talks to
/// [StormTerminal] and [StormTerminalView], so the emulator can be forked or
/// replaced without touching a screen. The revisit triggers are AC-Q5's: no
/// release for twelve months, or a phone that cannot render a burst.
class StormTerminal {
  StormTerminal() {
    _bytes = const Utf8Decoder(allowMalformed: true).startChunkedConversion(
      StringConversionSink.fromStringSink(_TerminalWriter(_terminal)),
    );
    _terminal.onOutput = (data) {
      if (_writingOutput && _colourAnswer.hasMatch(data)) {
        _reply(data, colour: true);
        // A replayed query's answer never reached the program; what the
        // host remembers it was told is better than this device's guess.
        if (_answeredHere || _remembered == null) answered.value = _target;
      } else if (_writingOutput || _schemeReport.hasMatch(data)) {
        _reply(data);
      } else {
        _emit(data);
      }
    };
    _terminal.onResize = (cols, rows, _, _) {
      if (cols > _widest) _widest = cols;
      onResize?.call(cols, rows);
    };
    _terminal.onTitleChange = (t) => title.value = t;
  }

  /// The window title the agent last set (OSC 0/2), raw. Claude Code and
  /// OpenCode name the conversation there once it has a topic.
  final title = ValueNotifier<String?>(null);

  final _terminal = Terminal(maxLines: 10000);

  /// Output arrives as bytes, and a stream event can end in the middle of a
  /// multibyte character. A chunked decoder carries the partial sequence into
  /// the next chunk instead of printing a replacement glyph.
  late ByteConversionSink _bytes;

  /// What the person typed, as the bytes the agent should receive.
  void Function(Uint8List bytes)? onInput;

  /// The terminal's answer to a query in the output it is drawing (device
  /// attributes, cursor position, colours, keyboard mode). Unset, it goes to
  /// [onInput]. Kept apart because an answer to a query replayed from
  /// history reaches the agent as typed text (decision 132, fix b).
  void Function(Uint8List bytes)? onReply;

  /// True while output is being drawn: whatever the emulator sends then is a
  /// reply, never a keystroke.
  bool _writingOutput = false;

  /// A DEC 2031 colour-scheme report, sent on a theme change outside any
  /// output: still a reply, never a keystroke, so it is gated like one.
  static final _schemeReport = RegExp(r'^\x1b\[\?997;[12]n$');

  /// An answer to the program's OSC 10 or 11 query.
  static final _colourAnswer = RegExp(r'^\x1b\]1[01];');

  /// The brightness Storm last answered the program's OSC 10/11 query with,
  /// null before any (decision 157c). An agent's view shows it: the program
  /// drew for it. Here, not in the view, so it outlives a remount.
  final answered = ValueNotifier<Brightness?>(null);

  /// This terminal's own colour answer reached the program, so what its
  /// host remembers is no news from then on.
  bool _answeredHere = false;
  Brightness? _remembered;
  bool _colourPending = false;
  bool _schemeReportPending = false;

  /// What the session's host says its program was last answered with
  /// (157g), from whichever device answered: shown until this terminal
  /// answers, so a reload or another device draws what the program drew
  /// for.
  void remembered(String? brightness) {
    final b = switch (brightness) {
      'light' => Brightness.light,
      'dark' => Brightness.dark,
      _ => null,
    };
    if (b == null) return;
    _remembered = b;
    if (!_answeredHere) answered.value = b;
  }

  /// The reply [onReply] is being handed went out to the program.
  void replySent() {
    if (_colourPending) _answeredHere = true;
    if (_schemeReportPending) _catchUpSentFor = _target;
  }

  /// What an answer gives now: the view's target, set as it builds.
  Brightness _target = Brightness.dark;

  /// What the session's host last said the program's DEC 2031 (colour-
  /// scheme report) mode was (the theme catch-up fix), from whichever
  /// device last reported a session status: a fallback for when this
  /// terminal's own emulator has not itself parsed the mode (its replay
  /// started after the program set it). This terminal's own parse, once it
  /// has one, is never stale, so it alone decides when it disagrees.
  bool? _rememberedSchemeReport;

  void rememberedSchemeReport(bool? enabled) {
    if (enabled != null) _rememberedSchemeReport = enabled;
  }

  bool get _schemeReportEnabled =>
      _terminal.reportColorSchemeMode || (_rememberedSchemeReport ?? false);

  /// The mismatch [catchUpTheme] last sent a report for, so it is not
  /// resent every time this is called before the program answers again.
  Brightness? _catchUpSentFor;

  /// The theme catch-up fix: when the program has DEC 2031 on and a known
  /// answer (this terminal's own, or the host's remembered one) disagrees
  /// with [_target], sends a colour-scheme report through the same gated
  /// path a live theme switch uses, so the program re-asks and this
  /// terminal's own answer catches up. **Never fires on a null answer**:
  /// before any answer, dark is correct by design (decision 157c), not a
  /// disagreement to resolve — otherwise a program that merely draws `CSI
  /// ?2031h` in replay, never having been asked anything, would be
  /// answered for output alone (the invariant decision 132 fix b exists
  /// for). A no-op once the answer catches up, with DEC 2031 off, or
  /// already sent for this mismatch — so a session shown again, or shown
  /// for the first time since the app's theme changed, asks at most once.
  /// Call on a view becoming visible, after any
  /// [remembered]/[rememberedSchemeReport].
  ///
  /// Built here, not [Terminal.reportColorSchemeChange], because that
  /// answers only for [_terminal]'s own DEC 2031 state — exactly the gap
  /// this closes when only the host's remembered state knows it is on.
  void catchUpTheme() {
    final known = answered.value;
    if (known == null || known == _target) {
      _catchUpSentFor = null;
      return;
    }
    if (!_schemeReportEnabled || _catchUpSentFor == _target) return;
    _reply(_schemeReportFor(_target));
  }

  static String _schemeReportFor(Brightness target) =>
      '\x1b[?997;${target == Brightness.light ? 2 : 1}n';

  void _reply(String data, {bool colour = false}) {
    final bytes = Uint8List.fromList(utf8.encode(data));
    // A scheme report is marked sent only once it is confirmed to leave —
    // via [replySent] once [onReply]'s gate lets it through, or here when
    // there is no gate at all — never pre-emptively: one gated out (a
    // hidden view, still replay) must be retried, not counted as done.
    final schemeReport = _schemeReport.hasMatch(data);
    final reply = onReply;
    if (reply == null) {
      if (colour) _answeredHere = true;
      if (schemeReport) _catchUpSentFor = _target;
      onInput?.call(bytes);
      return;
    }
    _colourPending = colour;
    _schemeReportPending = schemeReport;
    try {
      reply(bytes);
    } finally {
      _colourPending = false;
      _schemeReportPending = false;
    }
  }

  /// The view's size changed.
  void Function(int cols, int rows)? onResize;

  /// Asked on a paste chord or Ctrl+V: pastes the clipboard's image and says
  /// whether there was one (D15 AM45). Without an image the keys do what
  /// they always did.
  Future<bool> Function()? onImagePaste;

  /// An image the on-screen keyboard inserted, such as a GIF or sticker from
  /// Gboard (decision 87). Unset, the keyboard offers no images.
  void Function(Uint8List bytes)? onKeyboardImage;

  static const keyboardImageTypes = [
    'image/png',
    'image/jpeg',
    'image/gif',
    'image/webp',
  ];

  ContentInsertionConfiguration? get _insertion {
    final take = onKeyboardImage;
    if (take == null) return null;
    return ContentInsertionConfiguration(
      allowedMimeTypes: keyboardImageTypes,
      onContentInserted: (content) {
        final data = content.data;
        if (data != null && data.isNotEmpty) take(data);
      },
    );
  }

  /// The phone keys row's sticky modifiers. Each applies to the next key,
  /// typed or tapped on the row, and is then released, as on a phone's own
  /// shift. Listenable, because typing consumes one without the row knowing.
  final ctrlArmed = ValueNotifier(false);
  final shiftArmed = ValueNotifier(false);

  /// Sticky Ctrl: the next character typed becomes its control code.
  bool get stickyCtrl => ctrlArmed.value;
  set stickyCtrl(bool on) => ctrlArmed.value = on;

  /// Sticky Shift: Shift+Enter (a new line in agents), Shift+Tab (Claude
  /// Code's mode switch), Shift+arrows; a typed letter comes out upper case.
  bool get stickyShift => shiftArmed.value;
  set stickyShift(bool on) => shiftArmed.value = on;

  int get cols => _terminal.viewWidth;
  int get rows => _terminal.viewHeight;

  /// The widest this terminal has been: xterm2 never shrinks a line's cells.
  int _widest = 0;

  /// Roughly the bytes its cells hold: 16 a cell, at the line capacity of
  /// the widest it has been, over both buffers.
  int get estimatedBytes {
    final lines =
        _terminal.mainBuffer.lines.length + _terminal.altBuffer.lines.length;
    final widest = _widest > cols ? _widest : cols;
    return lines * 16 * lineCapacity(widest);
  }

  /// xterm2's `BufferLine` capacity for [cols]: 64, doubling below 256, then
  /// 256 in steps of 32.
  static int lineCapacity(int cols) {
    var capacity = 64;
    if (cols < 256) {
      while (capacity < cols) {
        capacity *= 2;
      }
    } else {
      capacity = 256;
      while (capacity < cols) {
        capacity += 32;
      }
    }
    return capacity;
  }

  void write(Uint8List bytes) {
    _writingOutput = true;
    try {
      _bytes.add(bytes);
    } finally {
      _writingOutput = false;
    }
  }

  /// Clears everything, as after a gap at the start of a view (freeze §14).
  void reset() {
    _bytes.close();
    _bytes = const Utf8Decoder(allowMalformed: true).startChunkedConversion(
      StringConversionSink.fromStringSink(_TerminalWriter(_terminal)),
    );
    _terminal.write('\x1bc');
  }

  /// A line the client writes itself, dimmed, never sent to the agent.
  void note(String text) => _terminal.write('\r\n\x1b[2m$text\x1b[0m\r\n');

  /// A key from the phone row, with whatever modifiers are armed.
  void key(TerminalKey key) {
    final ctrl = stickyCtrl, shift = stickyShift;
    // Released before the key goes out: its bytes come back through [_emit],
    // which must not apply them a second time.
    stickyCtrl = false;
    stickyShift = false;
    final arrow = _arrowFinal[key];
    if (arrow != null && (ctrl || shift)) {
      // xterm's modified arrow, `CSI 1;<1+shift+4·ctrl><A–D>`, built here:
      // xterm2 5.2.0's keytab lets an `AnyMod` record ignore its own
      // `-Shift`, so it sends Shift+Up as Ctrl+Up.
      final mod = 1 + (shift ? 1 : 0) + (ctrl ? 4 : 0);
      _emit('\x1b[1;$mod$arrow');
      return;
    }
    _terminal.keyInput(key, ctrl: ctrl, shift: shift);
  }

  static const _arrowFinal = {
    TerminalKey.arrowUp: 'A',
    TerminalKey.arrowDown: 'B',
    TerminalKey.arrowRight: 'C',
    TerminalKey.arrowLeft: 'D',
  };

  void escape() => key(TerminalKey.escape);

  /// Tab, or Shift+Tab with the sticky Shift armed: always the legacy bytes
  /// (decision 147b), which every agent reads, whatever keyboard protocol
  /// it asked for.
  void tab() {
    final shift = stickyShift;
    stickyCtrl = false;
    stickyShift = false;
    _send(shift ? backTabBytes : tabBytes);
  }

  /// Shift+Tab, `ESC [ Z`: Claude Code's mode switch.
  void backTab() {
    stickyCtrl = false;
    stickyShift = false;
    _send(backTabBytes);
  }

  static const tabBytes = '\t';
  static const backTabBytes = '\x1b[Z';

  void _send(String data) =>
      onInput?.call(Uint8List.fromList(utf8.encode(data)));

  /// How long after an Escape a Tab leaves the terminal instead of reaching
  /// the agent (decision 147b, WCAG 2.1.2).
  static const escapeThenTab = Duration(milliseconds: 1500);

  static final _modifiers = {
    LogicalKeyboardKey.shiftLeft,
    LogicalKeyboardKey.shiftRight,
    LogicalKeyboardKey.controlLeft,
    LogicalKeyboardKey.controlRight,
    LogicalKeyboardKey.altLeft,
    LogicalKeyboardKey.altRight,
    LogicalKeyboardKey.metaLeft,
    LogicalKeyboardKey.metaRight,
  };

  /// When the last key typed was an Escape, else null.
  DateTime? _escapedAt;

  /// The clock, for tests.
  @visibleForTesting
  DateTime Function() clock = DateTime.now;
  void up() => key(TerminalKey.arrowUp);
  void down() => key(TerminalKey.arrowDown);
  void left() => key(TerminalKey.arrowLeft);
  void right() => key(TerminalKey.arrowRight);

  void paste(String text) => _terminal.paste(text);

  /// Whether the agent has turned on the kitty keyboard protocol. Only then
  /// can a modified Enter or Backspace be told apart from a plain one.
  bool get kittyKeyboard => _terminal.kittyKeyboardMode != 0;

  /// The bytes to send instead of xterm2's encoding for a chord the legacy
  /// encoding cannot express, or null to let xterm2 encode it.
  ///
  /// Without the kitty protocol, Shift+Enter is `ESC O M` (keypad Enter, which
  /// agents read as submit) and Ctrl/Cmd+Backspace are a plain Backspace. So
  /// they send the readline equivalents a desktop terminal sends: LF for a new
  /// line, Ctrl+W for a word, Ctrl+U for the line. Once an agent has asked for
  /// kitty, xterm2's own encoding is exact and this steps aside. The Runtime
  /// Host is meant to negotiate that protocol (AM22, draft); this stays the
  /// path for programs that never ask for it.
  String? _legacyFallback(
    LogicalKeyboardKey key, {
    required bool shift,
    required bool control,
    required bool alt,
    required bool meta,
    required TargetPlatform platform,
  }) {
    if (kittyKeyboard || alt) return null;
    if (key == LogicalKeyboardKey.enter ||
        key == LogicalKeyboardKey.numpadEnter) {
      return shift && !control && !meta ? '\n' : null;
    }
    if (key == LogicalKeyboardKey.backspace) {
      if (control && !meta) return '\x17';
      // Cmd is Super elsewhere, which a terminal does not use for editing.
      if (meta && !control && platform == TargetPlatform.macOS) return '\x15';
    }
    return null;
  }

  KeyEventResult _onKeyEvent(FocusNode node, KeyEvent event) {
    final keys = HardwareKeyboard.instance;
    if (event.logicalKey == LogicalKeyboardKey.tab &&
        !keys.isControlPressed &&
        !keys.isAltPressed &&
        !keys.isMetaPressed) {
      return _onTab(node, event, shift: keys.isShiftPressed);
    }
    final chord = pasteChord(
      event.logicalKey,
      shift: keys.isShiftPressed,
      control: keys.isControlPressed,
      alt: keys.isAltPressed,
      meta: keys.isMetaPressed,
      platform: defaultTargetPlatform,
    );
    // Left unhandled so the browser makes its own paste event: the clipboard
    // API needs a secure context, which a plain-HTTP LAN page is not.
    if (web && chord == PasteChord.paste) {
      if (event is KeyDownEvent) _escapedAt = null;
      return KeyEventResult.skipRemainingHandlers;
    }
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    // A modifier on its own is no next key: Escape, then Shift+Tab, leaves
    // backwards.
    if (event is KeyDownEvent && !_modifiers.contains(event.logicalKey)) {
      _escapedAt = event.logicalKey == LogicalKeyboardKey.escape
          ? clock()
          : null;
    }
    final ask = onImagePaste;
    // A held paste chord repeats. The repeat must not reach the terminal as
    // a bare ^V: the agent would look for an image in the host's own
    // clipboard and say there is none, while this paste is still in flight.
    if (chord != null && ask != null && !web && event is KeyRepeatEvent) {
      return KeyEventResult.handled;
    }
    if (chord != null && ask != null && !web && event is KeyDownEvent) {
      () async {
        if (await ask()) return;
        if (chord == PasteChord.paste) {
          final text = (await Clipboard.getData(Clipboard.kTextPlain))?.text;
          if (text != null && text.isNotEmpty) paste(text);
        } else {
          _emit('\x16');
        }
      }();
      return KeyEventResult.handled;
    }
    final bytes = _legacyFallback(
      event.logicalKey,
      shift: keys.isShiftPressed,
      control: keys.isControlPressed,
      alt: keys.isAltPressed,
      meta: keys.isMetaPressed,
      platform: defaultTargetPlatform,
    );
    if (bytes == null) return KeyEventResult.ignored;
    _emit(bytes);
    return KeyEventResult.handled;
  }

  /// **Tab and Shift+Tab belong to the agent while the terminal has focus**
  /// (decision 147b): Shift+Tab is Claude Code's mode switch, Tab its
  /// completion. The terminal sends them itself, before xterm2 or Flutter's
  /// focus traversal sees them, on every platform. The way out is Escape,
  /// then Tab (or Shift+Tab) within [escapeThenTab]: the Escape still
  /// reaches the agent, and the Tab moves focus instead of being sent.
  KeyEventResult _onTab(FocusNode node, KeyEvent event, {required bool shift}) {
    if (event is KeyUpEvent) return KeyEventResult.handled;
    final escaped = _escapedAt;
    _escapedAt = null;
    if (event is KeyDownEvent &&
        escaped != null &&
        clock().difference(escaped) <= escapeThenTab) {
      if (shift) {
        node.previousFocus();
      } else {
        node.nextFocus();
      }
      return KeyEventResult.handled;
    }
    stickyCtrl = false;
    stickyShift = false;
    _send(shift ? backTabBytes : tabBytes);
    return KeyEventResult.handled;
  }

  void _emit(String data) {
    if (stickyShift && data.length == 1) {
      stickyShift = false;
      // The keyboard's Enter arrives as CR whatever the protocol (kitty
      // leaves plain Enter alone), so this is where Shift+Enter is made.
      if (data == '\r') {
        if (kittyKeyboard) {
          _terminal.keyInput(TerminalKey.enter, shift: true);
        } else {
          onInput?.call(Uint8List.fromList('\n'.codeUnits));
        }
        return;
      }
      data = data.toUpperCase();
    }
    if (stickyCtrl && data.length == 1) {
      final code = data.toUpperCase().codeUnitAt(0);
      // `@`, A–Z and `[ \ ] ^ _` have control codes; anything else is sent as
      // typed and still consumes the modifier, as a real Ctrl would.
      stickyCtrl = false;
      if (code >= 0x40 && code <= 0x5f) {
        onInput?.call(Uint8List.fromList([code & 0x1f]));
        return;
      }
    }
    onInput?.call(Uint8List.fromList(utf8.encode(data)));
  }

  /// Whether this runs in a browser. Tests set it.
  @visibleForTesting
  static bool web = kIsWeb;

  /// Which paste a key is, if any: the platform's own paste (Cmd+V on a Mac,
  /// Ctrl+Shift+V elsewhere), or Ctrl+V, which Claude Code uses for images.
  @visibleForTesting
  static PasteChord? pasteChord(
    LogicalKeyboardKey key, {
    required bool shift,
    required bool control,
    required bool alt,
    required bool meta,
    required TargetPlatform platform,
  }) {
    if (key != LogicalKeyboardKey.keyV || alt) return null;
    final mac = platform == TargetPlatform.macOS;
    if (mac && meta && !control && !shift) return PasteChord.paste;
    if (!mac && control && shift && !meta) return PasteChord.paste;
    if (control && !shift && !meta) return PasteChord.controlV;
    return null;
  }

  void dispose() {
    _bytes.close();
    ctrlArmed.dispose();
    shiftArmed.dispose();
    title.dispose();
    answered.dispose();
  }

  /// What a person typing [data] would send, for tests.
  @visibleForTesting
  void typeForTest(String data) => _emit(data);

  /// The screen's text, for tests.
  @visibleForTesting
  String get textForTest => _terminal.buffer.getText();

  /// The emulator itself, for tests that check its modes.
  @visibleForTesting
  Terminal get emulatorForTest => _terminal;

  /// The link at [column] of buffer row [row], for tests.
  @visibleForTesting
  Uri? linkAtForTest(int column, int row) =>
      terminalLinkAt(_terminal, CellOffset(column, row));
}

/// Forwards each decoded chunk to the terminal as it arrives.
///
/// **Not `StringConversionSink.withCallback`**, which buffers everything and
/// calls back once, on close: output would never reach the screen.
class _TerminalWriter implements StringSink {
  _TerminalWriter(this._terminal);

  final Terminal _terminal;

  @override
  void write(Object? object) => _terminal.write('$object');

  @override
  void writeAll(Iterable<Object?> objects, [String separator = '']) =>
      _terminal.write(objects.join(separator));

  @override
  void writeCharCode(int charCode) =>
      _terminal.write(String.fromCharCode(charCode));

  @override
  void writeln([Object? object = '']) => _terminal.write('$object\n');
}

enum PasteChord { paste, controlV }

/// The terminal widget, themed from the tokens and sized by [prefs].
class StormTerminalView extends StatefulWidget {
  const StormTerminalView({
    super.key,
    required this.terminal,
    this.focusNode,
    this.autofocus = false,
    this.readOnly = false,
    this.phone = false,
    this.shell = false,
    this.prefs = const TerminalPrefs(),
    this.openLink = _launch,
  });

  final StormTerminal terminal;
  final FocusNode? focusNode;
  final bool autofocus;
  final bool readOnly;

  /// The phone's session: on `surface` rather than `bg`, with smaller text
  /// (handoff §3.6).
  final bool phone;

  /// A shell session follows the target brightness live; an agent shows the
  /// one its program was last answered with, dark before any (decision 157c).
  final bool shell;
  final TerminalPrefs prefs;

  /// Opens a link from the terminal (decision 122). Tests replace it.
  final Future<bool> Function(Uri link) openLink;

  static Future<bool> _launch(Uri link) => launchUrl(
    link,
    mode: LaunchMode.externalApplication,
    webOnlyWindowName: '_blank',
  );

  @override
  State<StormTerminalView> createState() => _StormTerminalViewState();
}

class _StormTerminalViewState extends State<StormTerminalView> {
  final _controller = TerminalController();
  final _viewKey = GlobalKey<TerminalViewState>();
  FocusNode? _ownFocus;

  FocusNode get _focus =>
      widget.focusNode ?? (_ownFocus ??= FocusNode(debugLabel: 'terminal'));

  StormTerminal get terminal => widget.terminal;

  /// The theme catch-up fix: whether this View has already asked [terminal]
  /// to catch up, so later rebuilds of the same mount leave a live switch
  /// to its own mechanism (decision 157c) instead of racing it.
  bool _catchUpChecked = false;

  @override
  void initState() {
    super.initState();
    terminal.answered.addListener(_onAnswered);
  }

  @override
  void didUpdateWidget(StormTerminalView old) {
    super.didUpdateWidget(old);
    if (old.terminal != terminal) {
      old.terminal.answered.removeListener(_onAnswered);
      terminal.answered.addListener(_onAnswered);
      // A different terminal is a different session shown: its own first
      // catch-up check, not the old one's.
      _catchUpChecked = false;
    }
  }

  void _onAnswered() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    terminal.answered.removeListener(_onAnswered);
    _controller.dispose();
    _ownFocus?.dispose();
    super.dispose();
  }

  // A press and drag never becomes xterm2's focusing tap; a touch still waits
  // for one, since focus opens a phone's keyboard.
  void _onPointerDown(PointerDownEvent event) {
    if (event.kind == PointerDeviceKind.mouse && !_focus.hasFocus) {
      _focus.requestFocus();
    }
  }

  void _copy() {
    final selection = _controller.selectionFor(terminal._terminal.buffer);
    if (selection == null) return;
    final text = terminal._terminal.buffer.getText(selection, true);
    copyToClipboard(text).then((_) {
      // The web's plain-HTTP fallback takes the browser's focus for its copy.
      if (kIsWeb && mounted && _focus.hasFocus) {
        _viewKey.currentState?.requestKeyboard();
      }
    });
  }

  /// A modifier-click opens the link under the pointer; a tap asks first,
  /// because a tap is also how the terminal is focused (decision 122).
  void _onTapUp(BuildContext context, TapUpDetails details, CellOffset cell) {
    final link = terminalLinkAt(terminal._terminal, cell);
    if (link == null) return;
    if (details.kind == PointerDeviceKind.touch) {
      _confirmLink(context, link, widget.openLink);
    } else if (_linkModifierPressed()) {
      widget.openLink(link);
    }
  }

  @override
  Widget build(BuildContext context) {
    final readOnly = widget.readOnly;
    final phone = widget.phone;
    final prefs = widget.prefs;
    final t = context.tokens;
    final appLight = Theme.of(context).brightness == Brightness.light;
    final target = prefs.target(Theme.of(context).brightness);
    terminal._target = target;
    // The theme catch-up fix: this view's first build (fresh, or after a
    // remount) is "became visible"; a live switch while it stays mounted
    // is decision 157c's own mechanism's, below, so this runs once per
    // mount rather than racing it every rebuild. A shell already follows
    // [target] live.
    if (!widget.shell && !_catchUpChecked) {
      _catchUpChecked = true;
      terminal.catchUpTheme();
    }
    final shown = widget.shell
        ? target
        : terminal.answered.value ?? Brightness.dark;
    final theme = _terminalTheme(t, shown, dark: !appLight, phone: phone);
    final answers = _terminalTheme(t, target, dark: !appLight, phone: phone);
    final style = TerminalStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: prefs.fontSize ?? (phone ? t.labelSize + 1 : t.codeSize),
      height: prefs.spacing.lineHeight,
      // Bright text on a light ground is the faint one (157b).
      drawBoldTextWithBrightColors: shown == Brightness.dark,
    );
    final base = EdgeInsets.all(t.sp * 1.25 * prefs.padding.scale);
    final cell = terminalCellHeight(style, MediaQuery.textScalerOf(context));
    // **The app's chords stop here while the terminal has focus** (freeze
    // §10, AM111). The terminal handles the keys it understands before they
    // bubble; this catches the ones it leaves, so Ctrl+K reaches the agent's
    // world or nobody, never Storm's search. Terminal-safe chords are taken
    // for the app before the terminal sees them.
    return Shortcuts(
      shortcuts: stormTerminalBlockList(),
      // Tab is the agent's here, so the way out is said (147b, WCAG 2.1.2).
      child: Actions(
        actions: {
          _CopyIntent: CallbackAction<_CopyIntent>(onInvoke: (_) => _copy()),
        },
        child: Semantics(
          container: true,
          label: readOnly ? terminalReadOnlyLabel : terminalLabel,
          child: LayoutBuilder(
            builder: (context, box) => Listener(
              onPointerDown: _onPointerDown,
              child: TerminalView(
                terminal._terminal,
                key: _viewKey,
                focusNode: _focus,
                autofocus: widget.autofocus,
                controller: _controller,
                shortcuts: terminalShortcuts(
                  meta: stormUsesMetaModifier,
                  web: StormTerminal.web,
                ),
                readOnly: readOnly,
                contentInsertionConfiguration: readOnly
                    ? null
                    : terminal._insertion,
                // Asked first; whatever it leaves goes to xterm2's own encoding.
                onKeyEvent: (node, event) {
                  final app = stormTerminalSafeKey(context, event);
                  if (app != null) return app;
                  return readOnly
                      ? KeyEventResult.ignored
                      : terminal._onKeyEvent(node, event);
                },
                theme: theme,
                reportTheme: answers,
                padding: evenTerminalPadding(base, box.maxHeight, cell),
                textStyle: style,
                keyboardAppearance: Brightness.dark,
                // Agents redraw on resize; the terminal follows its box.
                autoResize: true,
                onTapUp: (details, cell) => _onTapUp(context, details, cell),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Copy through [copyToClipboard]: xterm2's own uses `Clipboard.setData`,
/// which a plain-HTTP LAN page does not have.
class _CopyIntent extends Intent {
  const _CopyIntent();
}

/// AM116. Ctrl+A and Ctrl+V are the agent's. A paste [StormTerminal.pasteChord]
/// makes itself never reaches these. On the web the browser's own paste event
/// brings text and images.
@visibleForTesting
Map<ShortcutActivator, Intent> terminalShortcuts({
  required bool meta,
  required bool web,
}) => meta
    ? {
        const SingleActivator(LogicalKeyboardKey.keyC, meta: true):
            const _CopyIntent(),
        if (!web)
          const SingleActivator(LogicalKeyboardKey.keyV, meta: true):
              const PasteTextIntent(SelectionChangedCause.keyboard),
        const SingleActivator(LogicalKeyboardKey.keyA, meta: true):
            const SelectAllTextIntent(SelectionChangedCause.keyboard),
      }
    : {
        const SingleActivator(
          LogicalKeyboardKey.keyC,
          control: true,
          shift: true,
        ): const _CopyIntent(),
        if (!web)
          const SingleActivator(
            LogicalKeyboardKey.keyV,
            control: true,
            shift: true,
          ): const PasteTextIntent(
            SelectionChangedCause.keyboard,
          ),
      };

/// The terminal's accessible name: what it is, and how keyboard focus
/// leaves it, since Tab and Shift+Tab go to the agent (decision 147b).
const terminalLabel =
    'Agent terminal. Tab and Shift+Tab go to the agent. To leave, press '
    'Escape, then Tab.';
const terminalReadOnlyLabel = 'Agent terminal, ended.';

/// xterm2's link modifier: Cmd on macOS, Ctrl elsewhere.
bool _linkModifierPressed() => switch (defaultTargetPlatform) {
  TargetPlatform.macOS => HardwareKeyboard.instance.isMetaPressed,
  _ => HardwareKeyboard.instance.isControlPressed,
};

/// Agent output is untrusted, so only these open (decision 122).
const _linkSchemes = {'http', 'https', 'mailto'};

final _plainUrl = RegExp('(?:https?://|mailto:)[^\\s<>"\'`\uFFFD]+');

/// The link at [cell] (decision 122): the OSC 8 target there, else a plain
/// URL in the cell's wrapped line whose span covers it. Null unless the
/// scheme is http, https or mailto.
@visibleForTesting
Uri? terminalLinkAt(Terminal terminal, CellOffset cell) {
  final target = terminal.hyperlinkAt(cell);
  if (target != null) return _allowedLink(target);
  final lines = terminal.buffer.lines;
  if (cell.y < 0 || cell.y >= lines.length) return null;
  var first = cell.y;
  while (first > 0 && lines[first].isWrapped) {
    first--;
  }
  var last = cell.y;
  while (last + 1 < lines.length && lines[last + 1].isWrapped) {
    last++;
  }
  // One UTF-16 unit per cell, so an index in the text is a cell: an empty
  // cell reads as a space, anything outside the BMP as U+FFFD.
  final text = StringBuffer();
  var at = 0;
  for (var y = first; y <= last; y++) {
    final line = lines[y];
    if (y == cell.y) at = text.length + cell.x;
    for (var x = 0; x < line.length; x++) {
      final code = line.getCodePoint(x);
      text.writeCharCode(switch (code) {
        0 => 0x20,
        > 0xffff => 0xfffd,
        _ => code,
      });
    }
  }
  for (final match in _plainUrl.allMatches(text.toString())) {
    final url = _trimUrl(match[0]!);
    if (at >= match.start && at < match.start + url.length) {
      return _allowedLink(url);
    }
  }
  return null;
}

Uri? _allowedLink(String text) {
  final uri = Uri.tryParse(text);
  if (uri == null || !_linkSchemes.contains(uri.scheme.toLowerCase())) {
    return null;
  }
  if (uri.scheme != 'mailto' && uri.host.isEmpty) return null;
  return uri;
}

/// Prose punctuation after a URL, and a closing bracket it never opened.
String _trimUrl(String url) {
  while (url.isNotEmpty) {
    final last = url[url.length - 1];
    final opener = switch (last) {
      ')' => '(',
      ']' => '[',
      '}' => '{',
      _ => null,
    };
    final unbalanced =
        opener != null &&
        last.allMatches(url).length > opener.allMatches(url).length;
    if (!'.,;:!?'.contains(last) && !unbalanced) break;
    url = url.substring(0, url.length - 1);
  }
  return url;
}

/// The touch path: the link in full, then Open or Copy.
Future<void> _confirmLink(
  BuildContext context,
  Uri link,
  Future<bool> Function(Uri) openLink,
) {
  final messenger = ScaffoldMessenger.maybeOf(context);
  return showStormSheet<void>(
    context: context,
    title: 'Link',
    heightFactor: 0.4,
    builder: (sheet) {
      final t = sheet.tokens;
      return Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SelectableText(
            '$link',
            style: TextStyle(
              fontFamily: StormTokens.monoFamily,
              fontSize: t.labelSize,
              color: t.text,
            ),
          ),
          SizedBox(height: t.sp * 2),
          Row(
            children: [
              Expanded(
                child: StormButton.outline(
                  label: 'Copy',
                  expand: true,
                  onPressed: () async {
                    await Clipboard.setData(ClipboardData(text: '$link'));
                    if (sheet.mounted) Navigator.of(sheet).pop();
                    messenger?.showSnackBar(
                      const SnackBar(content: Text('Link copied')),
                    );
                  },
                ),
              ),
              SizedBox(width: t.sp),
              Expanded(
                child: StormButton.primary(
                  label: 'Open',
                  expand: true,
                  onPressed: () {
                    Navigator.of(sheet).pop();
                    openLink(link);
                  },
                ),
              ),
            ],
          ),
        ],
      );
    },
  );
}

/// [base] with the height left under the last whole row split above and
/// below it, so the grid sits centred instead of leaving a gap at the bottom.
@visibleForTesting
EdgeInsets evenTerminalPadding(EdgeInsets base, double height, double cell) {
  final inner = height - base.vertical;
  if (!inner.isFinite || cell <= 0 || inner < cell) return base;
  // A hair under the remainder, so xterm2's own `~/` still fits every row.
  final spare = math.max(0.0, inner - (inner / cell).floor() * cell - 0.01);
  return base.copyWith(
    top: base.top + spare / 2,
    bottom: base.bottom + spare / 2,
  );
}

final _cellHeights = <(TerminalStyle, TextScaler), double>{};

/// A row's height, measured as xterm2's painter measures it.
@visibleForTesting
double terminalCellHeight(TerminalStyle style, TextScaler scaler) =>
    _cellHeights.putIfAbsent((style, scaler), () {
      final text = style.toTextStyle();
      final run = text.getTextStyle(textScaler: scaler);
      var height = 0.0;
      for (var c = 0x21; c <= 0x7e; c++) {
        final builder = ui.ParagraphBuilder(text.getParagraphStyle())
          ..pushStyle(run)
          ..addText(String.fromCharCode(c));
        final paragraph = builder.build()
          ..layout(const ui.ParagraphConstraints(width: double.infinity));
        height = math.max(height, paragraph.height);
        paragraph.dispose();
      }
      return height;
    });

/// The theme a view of [brightness] draws with (decisions 157a, 160).
///
/// A view in the app's own brightness wears the theme's derived terminal
/// palette; one in the other brightness wears the family's other member's,
/// or Storm's when the family has only the one (SlowFlow is light only). A
/// dark phone view sits on the surface rather than the page.
TerminalTheme _terminalTheme(
  StormTokens t,
  Brightness brightness, {
  required bool dark,
  required bool phone,
}) {
  final p = terminalPaletteFor(t, brightness);
  final background = dark && brightness == Brightness.dark && phone
      ? t.surface
      : p.background;
  return _themes.putIfAbsent((
    p,
    background,
    t.accent,
  ), () => _theme(p, background, t.accent));
}

/// The terminal palette of [t]'s family in [brightness].
TerminalPalette terminalPaletteFor(StormTokens t, Brightness brightness) {
  if (t.brightness == brightness) return t.terminal;
  final family = ThemeLibrary.family(t.variant.family);
  final other = family.has(brightness)
      ? family.variantFor(brightness)
      : brightness == Brightness.dark
      ? StormPreset.stormDark
      : StormPreset.stormLight;
  return StormTokens.from(other).terminal;
}

/// A light view's ground in a light app.
Color lightTerminalBackground(StormTokens t) => lightTerminalGround(t.bg);

// xterm2 reports a colour-scheme change to the agent whenever the view gets a
// theme that is not identical, and TerminalTheme has no value equality.
final _themes = <(TerminalPalette, Color, Color), TerminalTheme>{};

TerminalTheme _theme(TerminalPalette palette, Color background, Color accent) {
  final base = TerminalThemes.defaultTheme;
  final foreground = palette.foreground;
  final p = palette.ansi;
  return TerminalTheme(
    cursor: foreground.withValues(alpha: 0.7),
    selection: accent.withValues(alpha: 0.35),
    foreground: foreground,
    background: background,
    black: p[0],
    red: p[1],
    green: p[2],
    yellow: p[3],
    blue: p[4],
    magenta: p[5],
    cyan: p[6],
    white: p[7],
    brightBlack: p[8],
    brightRed: p[9],
    brightGreen: p[10],
    brightYellow: p[11],
    brightBlue: p[12],
    brightMagenta: p[13],
    brightCyan: p[14],
    brightWhite: p[15],
    searchHitBackground: base.searchHitBackground,
    searchHitBackgroundCurrent: base.searchHitBackgroundCurrent,
    searchHitForeground: base.searchHitForeground,
  );
}
