import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:xterm2/xterm.dart';

import '../keyboard/storm_shortcut_maps.dart';
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
    _terminal.onOutput = _emit;
    _terminal.onResize = (cols, rows, _, _) => onResize?.call(cols, rows);
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

  /// The view's size changed.
  void Function(int cols, int rows)? onResize;

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

  void write(Uint8List bytes) => _bytes.add(bytes);

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
  void tab() => key(TerminalKey.tab);
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

  KeyEventResult _onKeyEvent(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    final keys = HardwareKeyboard.instance;
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

  void dispose() {
    _bytes.close();
    ctrlArmed.dispose();
    shiftArmed.dispose();
    title.dispose();
  }

  /// What a person typing [data] would send, for tests.
  @visibleForTesting
  void typeForTest(String data) => _emit(data);

  /// The screen's text, for tests.
  @visibleForTesting
  String get textForTest => _terminal.buffer.getText();
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

/// The terminal widget, themed from the tokens.
class StormTerminalView extends StatelessWidget {
  const StormTerminalView({
    super.key,
    required this.terminal,
    this.focusNode,
    this.autofocus = false,
    this.readOnly = false,
    this.padding,
    this.fontSize,
    this.lineHeight = 1.2,
    this.surface = false,
  });

  /// The handoff's terminal sets 1.7 (§3.6); xterm2's own default is 1.2.
  final double lineHeight;

  final StormTerminal terminal;
  final FocusNode? focusNode;
  final bool autofocus;
  final bool readOnly;

  /// Around the character grid; `sp` on every side when null. The padding is
  /// drawn in the view's colour, not the agent's, so a full-screen agent that
  /// paints its own background (OpenCode) shows it as a frame.
  final EdgeInsets? padding;
  final double? fontSize;

  /// On `surface` rather than `bg` (the phone's session, handoff §3.6).
  final bool surface;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final base = TerminalThemes.defaultTheme;
    // **Dark in both themes.** Agents and the ANSI palette assume a dark
    // background; on the light theme's white code plate, an agent's white text
    // would vanish. Both colours still come from the tokens: the light theme's
    // off-black text and off-white page, swapped.
    final dark = Theme.of(context).brightness == Brightness.dark;
    final background = dark ? (surface ? t.surface : t.bg) : t.text;
    final foreground = dark ? t.text : t.bg;
    final theme = TerminalTheme(
      cursor: foreground.withValues(alpha: 0.7),
      selection: t.accent.withValues(alpha: 0.35),
      foreground: foreground,
      background: background,
      black: base.black,
      white: base.white,
      red: base.red,
      green: base.green,
      yellow: base.yellow,
      blue: base.blue,
      magenta: base.magenta,
      cyan: base.cyan,
      brightBlack: base.brightBlack,
      brightRed: base.brightRed,
      brightGreen: base.brightGreen,
      brightYellow: base.brightYellow,
      brightBlue: base.brightBlue,
      brightMagenta: base.brightMagenta,
      brightCyan: base.brightCyan,
      brightWhite: base.brightWhite,
      searchHitBackground: base.searchHitBackground,
      searchHitBackgroundCurrent: base.searchHitBackgroundCurrent,
      searchHitForeground: base.searchHitForeground,
    );
    // **The app's chords stop here while the terminal has focus** (freeze
    // §10). The terminal handles the keys it understands before they bubble;
    // this catches the ones it leaves, so Ctrl+K reaches the agent's world or
    // nobody, never Storm's search.
    return Shortcuts(
      shortcuts: {
        for (final activator in stormGlobalShortcuts().keys)
          activator: const DoNothingAndStopPropagationIntent(),
      },
      child: TerminalView(
        terminal._terminal,
        focusNode: focusNode,
        autofocus: autofocus,
        readOnly: readOnly,
        // Asked first; whatever it leaves goes to xterm2's own encoding.
        onKeyEvent: readOnly ? null : terminal._onKeyEvent,
        theme: theme,
        padding: padding ?? EdgeInsets.all(t.sp),
        textStyle: TerminalStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: fontSize ?? t.labelSize + 1,
          height: lineHeight,
        ),
        keyboardAppearance: Brightness.dark,
        // Agents redraw on resize; the terminal follows its box.
        autoResize: true,
      ),
    );
  }
}
