import 'dart:convert';

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
  }

  final _terminal = Terminal(maxLines: 10000);

  /// Output arrives as bytes, and a stream event can end in the middle of a
  /// multibyte character. A chunked decoder carries the partial sequence into
  /// the next chunk instead of printing a replacement glyph.
  late ByteConversionSink _bytes;

  /// What the person typed, as the bytes the agent should receive.
  void Function(Uint8List bytes)? onInput;

  /// The view's size changed.
  void Function(int cols, int rows)? onResize;

  /// The phone keys row's sticky Ctrl: the next character typed becomes its
  /// control code.
  bool stickyCtrl = false;

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

  void key(TerminalKey key, {bool ctrl = false}) =>
      _terminal.keyInput(key, ctrl: ctrl);

  void escape() => key(TerminalKey.escape);
  void tab() => key(TerminalKey.tab);
  void up() => key(TerminalKey.arrowUp);
  void down() => key(TerminalKey.arrowDown);
  void left() => key(TerminalKey.arrowLeft);
  void right() => key(TerminalKey.arrowRight);

  void paste(String text) => _terminal.paste(text);

  void _emit(String data) {
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

  void dispose() => _bytes.close();

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
  });

  final StormTerminal terminal;
  final FocusNode? focusNode;
  final bool autofocus;
  final bool readOnly;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final base = TerminalThemes.defaultTheme;
    // **Dark in both themes.** Agents and the ANSI palette assume a dark
    // background; on the light theme's white code plate, an agent's white text
    // would vanish. Both colours still come from the tokens: the light theme's
    // off-black text and off-white page, swapped.
    final dark = Theme.of(context).brightness == Brightness.dark;
    final background = dark ? t.bg : t.text;
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
        theme: theme,
        padding: EdgeInsets.all(t.sp),
        textStyle: TerminalStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.labelSize + 1,
        ),
        keyboardAppearance: Brightness.dark,
        // Agents redraw on resize; the terminal follows its box.
        autoResize: true,
      ),
    );
  }
}
