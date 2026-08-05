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
