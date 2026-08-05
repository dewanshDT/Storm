import 'package:flutter/material.dart';

import 'markdown_theme.dart';
import 'markdown_tokenizer.dart';

/// A [TextEditingController] that renders markdown with live styling.
///
/// The performance problem this class exists to solve is flutter#114158:
/// [buildTextSpan] is invoked on *every* value change including bare caret
/// movement and selection drags, so a naive implementation re-tokenizes the
/// whole document dozens of times a second while you arrow around a long note.
///
/// Two defences:
///
///  1. **Whole-span memo.** If the text and base style are unchanged since the
///     last build, the previously built [TextSpan] is returned outright. This
///     makes caret movement and selection free, which is the common case.
///  2. **Per-line span cache.** When the text *has* changed, lines are looked
///     up in a cache keyed by their own content, so a keystroke re-tokenizes
///     only the line it touched. Because cached spans hold text rather than
///     offsets, inserting a line shifts every following line's offsets without
///     invalidating any of them.
///
/// Above [maxStyledLines] the controller stops styling and returns a plain
/// span. Past that size the span tree itself is the bottleneck, and a
/// responsive unstyled editor beats a styled one that drops frames.
class StormMarkdownController extends TextEditingController {
  // Dart forbids private named parameters, and the field needs a custom setter
  // that invalidates the caches, so an initializing formal isn't available.
  StormMarkdownController({required MarkdownTheme theme, super.text})
    // ignore: prefer_initializing_formals
    : _theme = theme;

  static const int maxStyledLines = 5000;

  MarkdownTheme _theme;

  /// Changes the styling.
  ///
  /// Deliberately does **not** call `notifyListeners()`. To a
  /// [TextEditingController]'s listeners a notification means "the value
  /// changed", and callers set the theme from `build()` — so notifying here
  /// both lies about the text and fires during a build. Dropping the caches is
  /// enough: the rebuild that set the theme re-runs [buildTextSpan] anyway.
  set theme(MarkdownTheme value) {
    if (value == _theme) return;
    _theme = value;
    _dropCaches();
  }

  MarkdownTheme get theme => _theme;

  /// Hide markdown punctuation on lines the caret isn't on.
  ///
  /// This is Obsidian's Live Preview behaviour. The characters stay in the
  /// buffer — they have to, or every caret offset past them would be wrong —
  /// but they render at effectively zero size, so an unfocused document reads
  /// as prose. Move the caret onto a line and its syntax comes back at full
  /// size, which is exactly where you want to see it.
  bool revealOnActiveLine = true;

  Map<_LineKey, _CachedLine> _lineCache = {};
  String? _memoText;
  TextStyle? _memoStyle;
  TextSpan? _memoSpan;
  // The memo has to key on the revealed range too: the same text renders
  // differently depending on which line the caret is in.
  int _memoRevealStart = -1;
  int _memoRevealEnd = -1;

  /// Diagnostics for the M0 perf gate — how the last build was served.
  int lastLinesTokenized = 0;
  int lastLinesTotal = 0;
  bool lastServedFromMemo = false;
  bool lastDegraded = false;

  /// Discards memoised spans without claiming the text changed.
  void _dropCaches() {
    _lineCache = {};
    _memoText = null;
    _memoSpan = null;
  }

  // --- Editing ---------------------------------------------------------
  //
  // The formatting toolbar calls these and nothing else. Every one of them
  // computes the new text *and* the new selection and assigns `value` exactly
  // once: assigning `text` and `selection` separately fires two notifications,
  // and the intermediate state has a caret pointing into a buffer that no
  // longer matches it.

  /// Wraps the selection in [marker], or unwraps it if it is already wrapped.
  ///
  /// With a collapsed caret this inserts the pair and sits between them, which
  /// is what "turn bold on" means when there is nothing selected yet.
  void toggleInline(String marker) {
    final sel = selection;
    if (!sel.isValid) return;
    final t = text;
    final start = sel.start;
    final end = sel.end;

    if (start == end) {
      final already = _wrappedAt(t, start, end, marker);
      if (already) return _unwrap(t, start, end, marker);
      return _apply(
        t.substring(0, start) + marker + marker + t.substring(start),
        TextSelection.collapsed(offset: start + marker.length),
      );
    }

    // Markers just outside the selection — you selected the word, not the
    // asterisks around it.
    if (_wrappedAt(t, start, end, marker)) {
      return _unwrap(t, start, end, marker);
    }

    // Markers inside the selection — you selected `**word**` whole.
    final selected = t.substring(start, end);
    if (selected.length >= marker.length * 2 &&
        selected.startsWith(marker) &&
        selected.endsWith(marker)) {
      final inner = selected.substring(
        marker.length,
        selected.length - marker.length,
      );
      return _apply(
        t.substring(0, start) + inner + t.substring(end),
        TextSelection(baseOffset: start, extentOffset: start + inner.length),
      );
    }

    _apply(
      t.substring(0, start) + marker + selected + marker + t.substring(end),
      TextSelection(
        baseOffset: start + marker.length,
        extentOffset: end + marker.length,
      ),
    );
  }

  bool _wrappedAt(String t, int start, int end, String marker) {
    final n = marker.length;
    if (start - n < 0 || end + n > t.length) return false;
    return t.substring(start - n, start) == marker &&
        t.substring(end, end + n) == marker;
  }

  void _unwrap(String t, int start, int end, String marker) {
    final n = marker.length;
    _apply(
      t.substring(0, start - n) + t.substring(start, end) + t.substring(end + n),
      start == end
          ? TextSelection.collapsed(offset: start - n)
          : TextSelection(baseOffset: start - n, extentOffset: end - n),
    );
  }

  /// Sets the block prefix — `## `, `> `, `- `, `1. `, `- [ ] ` — on every line
  /// the selection touches, replacing whatever prefix was there.
  ///
  /// Pass null to strip whatever is there.
  ///
  /// [toggle] decides what re-applying a prefix a line already has means, and
  /// the answer depends on how the control is shaped. A lone on/off button —
  /// bullets, quote — has nowhere else to say "off", so it toggles. A *picker*
  /// with an explicit Paragraph entry must not: choosing "Heading 1" on a line
  /// that is already H1 means "make this H1", and quietly deleting the `#` is
  /// how the heading button came to look like it did nothing at all.
  void setBlockPrefix(String? prefix, {bool toggle = true}) {
    final sel = selection;
    if (!sel.isValid) return;
    final t = text;
    final blockStart = _lineStartAt(t, sel.start);
    final blockEnd = _lineEndAt(t, sel.end);
    final lines = t.substring(blockStart, blockEnd).split('\n');

    final stripped = [for (final line in lines) _splitPrefix(line)];

    // An ordered list is a *sequence*, not a repeated string: `1. ` on every
    // line is what a numbered list looks like when nobody counted. Numbering
    // continues from the item above the selection, so extending a list picks
    // up where it left off instead of restarting.
    final ordered = prefix != null && orderedMarker.hasMatch(prefix);
    final firstNumber = ordered ? _orderedNumberBefore(t, blockStart) : 1;

    final removing = prefix == null ||
        (toggle &&
            (ordered
                ? stripped.every((p) => orderedMarker.hasMatch(p.marker))
                : stripped.every((p) => p.marker == prefix)));

    String markerFor(int index) {
      if (removing) return '';
      if (!ordered) return prefix;
      // Keep whichever delimiter the caller asked for — `1.` and `1)` are both
      // ordered lists, and switching one under the user would be rude.
      final delimiter = prefix.contains(')') ? ')' : '.';
      return '${firstNumber + index}$delimiter ';
    }

    final rewritten = [
      for (final (i, p) in stripped.indexed) '${p.indent}${markerFor(i)}${p.body}',
    ].join('\n');

    // The caret keeps its place within the first line's *content*, which is
    // where it visually was — not its raw offset, which the prefix just moved.
    final firstDelta = markerFor(0).length - stripped.first.marker.length;
    _apply(
      t.substring(0, blockStart) + rewritten + t.substring(blockEnd),
      sel.isCollapsed
          ? TextSelection.collapsed(
              offset: (sel.start + firstDelta).clamp(
                blockStart,
                blockStart + rewritten.length,
              ),
            )
          : TextSelection(
              baseOffset: blockStart,
              extentOffset: blockStart + rewritten.length,
            ),
    );
  }

  /// Inserts `[[]]` with the caret between the brackets, or wraps the
  /// selection into a link to a note of that name.
  ///
  /// There is no completion behind this yet; it is a bracket-matching
  /// convenience, and deliberately named for what it does.
  void insertWikilink() {
    final sel = selection;
    if (!sel.isValid) return;
    final t = text;
    final inner = t.substring(sel.start, sel.end);
    _apply(
      '${t.substring(0, sel.start)}[[$inner]]${t.substring(sel.end)}',
      inner.isEmpty
          ? TextSelection.collapsed(offset: sel.start + 2)
          : TextSelection(
              baseOffset: sel.start + 2,
              extentOffset: sel.start + 2 + inner.length,
            ),
    );
  }

  void _apply(String next, TextSelection nextSelection) {
    value = TextEditingValue(
      text: next,
      selection: nextSelection,
      composing: TextRange.empty,
    );
  }
  @override
  TextSpan buildTextSpan({
    required BuildContext context,
    TextStyle? style,
    required bool withComposing,
  }) {
    final baseStyle = style ?? const TextStyle();
    final composing =
        withComposing &&
            !value.composing.isCollapsed &&
            value.isComposingRangeValid
        ? value.composing
        : null;

    // Computed from offsets, not by splitting: this runs on *every* call,
    // including memo hits, and walking the whole document here cost more than
    // the memo saves.
    final (revealFrom, revealTo) = _revealedRange();

    // (1) Whole-span memo. Skipped while an IME composition is live, since the
    // composing underline moves independently of the text. Caret movement
    // *within* a line still hits this; only crossing a line boundary rebuilds.
    if (composing == null &&
        _memoSpan != null &&
        _memoText == text &&
        _memoStyle == baseStyle &&
        _memoRevealStart == revealFrom &&
        _memoRevealEnd == revealTo) {
      lastServedFromMemo = true;
      return _memoSpan!;
    }
    lastServedFromMemo = false;

    final resolvedTheme = _theme.base == baseStyle
        ? _theme
        : MarkdownTheme(
            base: baseStyle.merge(_theme.base),
            markerColor: _theme.markerColor,
            accent: _theme.accent,
            codeColor: _theme.codeColor,
            codeBackground: _theme.codeBackground,
            mutedColor: _theme.mutedColor,
            highlightBackground: _theme.highlightBackground,
          );

    final lines = text.split('\n');
    lastLinesTotal = lines.length;
    lastLinesTokenized = 0;

    // (3) Degrade rather than stutter.
    if (lines.length > maxStyledLines) {
      lastDegraded = true;
      final span = TextSpan(text: text, style: baseStyle);
      _memoText = text;
      _memoStyle = baseStyle;
      _memoSpan = span;
      return span;
    }
    lastDegraded = false;

    final children = <InlineSpan>[];
    // Retain only lines still present, so the cache can't grow without bound
    // across a long editing session.
    final nextCache = <_LineKey, _CachedLine>{};

    var ctx = const BlockContext();
    var offset = 0;

    for (var i = 0; i < lines.length; i++) {
      final line = lines[i];
      final lineEnd = offset + line.length;
      // Offset overlap rather than line index — see _revealedRange.
      final revealed =
          !revealOnActiveLine || (lineEnd >= revealFrom && offset <= revealTo);
      final key = _LineKey(line, ctx, i == 0, revealed);

      // A line overlapping the IME composing range must be rebuilt with the
      // composing underline, and must not pollute the cache.
      final touchesComposing =
          composing != null &&
          composing.start <= lineEnd &&
          composing.end >= offset;

      final _CachedLine built;
      if (touchesComposing) {
        built = _buildLine(
          line,
          ctx,
          i,
          resolvedTheme,
          revealed: revealed,
          composing: composing,
          lineOffset: offset,
        );
        lastLinesTokenized++;
      } else {
        final cached = _lineCache[key] ?? nextCache[key];
        if (cached != null) {
          built = cached;
        } else {
          built = _buildLine(line, ctx, i, resolvedTheme, revealed: revealed);
          lastLinesTokenized++;
        }
        nextCache[key] = built;
      }

      children.addAll(built.spans);
      if (i != lines.length - 1) {
        children.add(TextSpan(text: '\n', style: resolvedTheme.base));
      }

      ctx = built.nextContext;
      offset = lineEnd + 1; // +1 for the '\n'
    }

    final span = TextSpan(style: baseStyle, children: children);

    _lineCache = nextCache;
    if (composing == null) {
      _memoText = text;
      _memoStyle = baseStyle;
      _memoSpan = span;
      _memoRevealStart = revealFrom;
      _memoRevealEnd = revealTo;
    } else {
      _memoSpan = null;
    }
    return span;
  }

  /// Tokenizes one line and converts its runs into styled spans.
  ///
  /// Tokenizes at offset 0 so the resulting spans carry text, not document
  /// positions — that is what makes them reusable after an edit elsewhere
  /// shifts this line up or down. The resulting [BlockContext] is cached with
  /// the spans so a cache hit costs no tokenization at all.
  /// The character range of the lines whose syntax should be shown: the line
  /// holding the caret, or every line a selection touches.
  ///
  /// Deliberately expressed in offsets and found by scanning to the nearest
  /// newline either side of the selection. Walking the document to convert
  /// this into line *indices* would be O(lines) on every single call —
  /// including the memo hits that are supposed to make caret movement free.
  (int, int) _revealedRange() {
    final sel = value.selection;
    if (!sel.isValid || !revealOnActiveLine) return (-1, -1);

    final from = sel.start <= 0 ? 0 : text.lastIndexOf('\n', sel.start - 1) + 1;
    var to = text.indexOf('\n', sel.end);
    if (to < 0) to = text.length;
    return (from, to);
  }

  _CachedLine _buildLine(
    String line,
    BlockContext ctx,
    int index,
    MarkdownTheme theme, {
    required bool revealed,
    TextRange? composing,
    int lineOffset = 0,
  }) {
    final result = tokenizeLine(line, 0, ctx, index);
    if (line.isEmpty) {
      return _CachedLine(const [], result.nextContext);
    }

    final tokens = result.tokens;
    final spans = <InlineSpan>[];

    // A line made only of punctuation would collapse to nothing if it were
    // hidden, leaving the caret nowhere to land. Keep those visible.
    final hasVisibleContent = tokens.any((t) => !_hiddenWhenInactive(t.kind));
    final hide = !revealed && hasVisibleContent;

    void emit(int start, int end, TokenKind kind) {
      if (start >= end) return;
      var style = hide && _hiddenWhenInactive(kind)
          ? theme.hiddenMarker
          : theme.styleFor(kind);
      if (composing != null) {
        // Underline only the composing slice; split if it partially covers.
        final cs = composing.start - lineOffset;
        final ce = composing.end - lineOffset;
        if (cs < end && ce > start) {
          final underlined = style.copyWith(
            decoration: TextDecoration.underline,
          );
          if (cs > start) {
            spans.add(TextSpan(text: line.substring(start, cs), style: style));
          }
          spans.add(
            TextSpan(
              text: line.substring(cs.clamp(start, end), ce.clamp(start, end)),
              style: underlined,
            ),
          );
          if (ce < end) {
            spans.add(TextSpan(text: line.substring(ce, end), style: style));
          }
          return;
        }
      }
      spans.add(TextSpan(text: line.substring(start, end), style: style));
    }

    var cursor = 0;
    for (final token in tokens) {
      if (token.start > cursor) {
        emit(cursor, token.start, TokenKind.text);
      }
      emit(token.start, token.end, token.kind);
      cursor = token.end;
    }
    if (cursor < line.length) {
      emit(cursor, line.length, TokenKind.text);
    }
    return _CachedLine(spans, result.nextContext);
  }
}

/// A line's rendered spans plus the block context it leaves behind.
class _CachedLine {
  const _CachedLine(this.spans, this.nextContext);

  final List<InlineSpan> spans;
  final BlockContext nextContext;
}

/// Syntax that disappears when the caret is elsewhere.
///
/// Only punctuation. List bullets stay — they read as bullets, not syntax —
/// and a link's URL goes so `[text](url)` reads as `text`.
bool _hiddenWhenInactive(TokenKind kind) =>
    kind == TokenKind.marker || kind == TokenKind.linkUrl;

/// Cache key for a line's spans.
///
/// Includes the inbound [BlockContext] because the same text renders
/// differently inside a fence, [isFirst] because only line 0 can open
/// frontmatter, and [revealed] because the caret's line shows its syntax.
class _LineKey {
  const _LineKey(this.line, this.context, this.isFirst, this.revealed);

  final String line;
  final BlockContext context;
  final bool isFirst;
  final bool revealed;

  @override
  bool operator ==(Object other) =>
      other is _LineKey &&
      other.line == line &&
      other.context == context &&
      other.isFirst == isFirst &&
      other.revealed == revealed;

  @override
  int get hashCode => Object.hash(line, context, isFirst, revealed);
}


/// A line split into indent, block marker, and the content after it.
class _Prefixed {
  const _Prefixed(this.indent, this.marker, this.body);

  final String indent;
  final String marker;
  final String body;
}

/// Task markers come first: `- [ ] x` is also a valid bullet, and matching it
/// as one would leave `[ ] ` stranded in the text.
final _blockMarker = RegExp(
  r'^(?:[-*+] \[[ xX]\] |#{1,6} |>+ ?|[-*+] |\d+[.)] )',
);
final _indent = RegExp(r'^[ \t]*');

_Prefixed _splitPrefix(String line) {
  final indent = _indent.firstMatch(line)![0]!;
  final rest = line.substring(indent.length);
  final m = _blockMarker.matchAsPrefix(rest);
  if (m == null) return _Prefixed(indent, '', rest);
  return _Prefixed(indent, m[0]!, rest.substring(m.end));
}

/// `1. `, `2) ` — an ordered-list marker, with its trailing space.
final orderedMarker = RegExp(r'^\d+[.)] ');

/// What number an ordered item inserted at [blockStart] should carry.
///
/// Continues the run immediately above it, so extending a list numbers 4, 5, 6
/// rather than restarting at 1. A blank line or anything unnumbered ends the
/// run, which is also where markdown itself starts a new list.
int _orderedNumberBefore(String t, int blockStart) {
  if (blockStart == 0) return 1;
  final previousEnd = blockStart - 1;
  final previousStart = _lineStartAt(t, previousEnd);
  final previous = t.substring(previousStart, previousEnd);
  final m = orderedMarker.matchAsPrefix(previous.trimLeft());
  if (m == null) return 1;
  return (int.tryParse(RegExp(r'\d+').firstMatch(m[0]!)![0]!) ?? 0) + 1;
}

int _lineStartAt(String t, int offset) {
  // `lastIndexOf` throws on a negative start, which offset 0 produces — a
  // selection anchored at the very first character of the buffer.
  if (offset <= 0) return 0;
  final i = t.lastIndexOf('\n', offset - 1);
  return i < 0 ? 0 : i + 1;
}

int _lineEndAt(String t, int offset) {
  final i = t.indexOf('\n', offset);
  return i < 0 ? t.length : i;
}

/// Where a `[[wikilink]]` sits, if the caret at [offset] is inside one.
///
/// Taps inside an editable TextField are consumed for caret placement, so a
/// TapGestureRecognizer on a span never fires. Instead the tap places the caret
/// as usual and the caller asks this what the caret landed in — the pointer
/// keeps its ordinary behaviour and nothing fights the selection gesture.
WikilinkHit? wikilinkAt(String text, int offset) {
  if (offset < 0 || offset > text.length) return null;
  final lineStart = _lineStartAt(text, offset);
  final lineEnd = _lineEndAt(text, offset);
  final line = text.substring(lineStart, lineEnd);

  for (final m in RegExp(r'\[\[([^\[\]\n]+)\]\]').allMatches(line)) {
    final start = lineStart + m.start;
    final end = lineStart + m.end;
    if (offset >= start && offset <= end) {
      return WikilinkHit(target: m[1]!.trim(), start: start, end: end);
    }
  }
  return null;
}

/// A `[[target]]` under the caret, in absolute buffer offsets.
class WikilinkHit {
  const WikilinkHit({
    required this.target,
    required this.start,
    required this.end,
  });

  final String target;
  final int start;
  final int end;
}
