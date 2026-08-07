import 'package:flutter/services.dart';

import 'storm_markdown_controller.dart';

/// Carries a list on when you press Enter inside one.
///
/// A [TextInputFormatter] rather than an `onChanged` hook, because this is the
/// one place that sees an edit *before* it lands and can rewrite the text and
/// the caret together. Doing it afterwards means a frame where the buffer and
/// the caret disagree, which is the failure the controller's editing methods
/// exist to avoid.
///
/// Two behaviours, both what every markdown editor does:
///
///  * Enter at the end of `- item` starts `- ` on the next line, numbering it
///    if the list was numbered.
///  * Enter on an item that is *empty* ends the list instead of adding another
///    empty bullet — otherwise there is no way out except backspacing.
class ListContinuationFormatter extends TextInputFormatter {
  const ListContinuationFormatter();

  @override
  TextEditingValue formatEditUpdate(
    TextEditingValue oldValue,
    TextEditingValue newValue,
  ) {
    final caret = newValue.selection.baseOffset;

    // Only a plain Enter keystroke. Anything else — a paste, an IME
    // composition, a deletion — is left exactly alone, because guessing at
    // intent there is how an autocorrect eats your text.
    if (!newValue.selection.isCollapsed ||
        caret <= 0 ||
        caret > newValue.text.length ||
        newValue.text.length != oldValue.text.length + 1 ||
        newValue.text[caret - 1] != '\n') {
      return newValue;
    }

    final continuation = listContinuation(newValue.text, caret);
    if (continuation == null) return newValue;
    return continuation;
  }
}

/// The value that pressing Enter at [caret] should produce, or null to leave
/// the edit alone. Separated from the formatter so it can be tested directly.
TextEditingValue? listContinuation(String text, int caret) {
  // The line that was just ended, i.e. everything before the new newline.
  final endedAt = caret - 1;
  // lastIndexOf throws on a negative start, which an Enter on the very first
  // line produces.
  final start = endedAt <= 0 ? 0 : text.lastIndexOf('\n', endedAt - 1) + 1;
  final ended = text.substring(start, endedAt);

  final indent = RegExp(r'^[ \t]*').firstMatch(ended)![0]!;
  final rest = ended.substring(indent.length);

  final marker = _listMarker(rest);
  if (marker == null) return null;

  final body = rest.substring(marker.length);
  if (body.trim().isEmpty) {
    // An empty item: Enter ends the list rather than adding another one.
    // The marker goes, and so does the newline just typed — the caret stays on
    // that now-blank line instead of leaving a stray empty line behind.
    return TextEditingValue(
      text: text.substring(0, start) + text.substring(caret),
      selection: TextSelection.collapsed(offset: start),
      composing: TextRange.empty,
    );
  }

  final next = '$indent${_nextMarker(marker)}';
  return TextEditingValue(
    text: text.substring(0, caret) + next + text.substring(caret),
    selection: TextSelection.collapsed(offset: caret + next.length),
    composing: TextRange.empty,
  );
}

/// The list marker a line opens with, or null if it isn't a list item.
///
/// Task markers are checked first: `- [ ] x` is also a valid bullet, and
/// matching it as one would leave `[ ] ` stranded in the continued line.
String? _listMarker(String rest) {
  final m = RegExp(
    r'^(?:[-*+] \[[ xX]\] |[-*+] |\d+[.)] )',
  ).matchAsPrefix(rest);
  return m?[0];
}

/// What the *next* item's marker looks like. Numbers advance; a checked task
/// continues as an unchecked one, since the new item hasn't been done yet.
String _nextMarker(String marker) {
  final ordered = orderedMarker.matchAsPrefix(marker);
  if (ordered != null) {
    final n = int.parse(RegExp(r'\d+').firstMatch(marker)![0]!);
    return marker.contains(')') ? '${n + 1}) ' : '${n + 1}. ';
  }
  final task = RegExp(r'^([-*+]) \[[ xX]\] ').matchAsPrefix(marker);
  if (task != null) return '${task[1]} [ ] ';
  return marker;
}
