/// Line-oriented markdown tokenizer for the Storm live-styling editor.
///
/// Deliberately *not* a markdown parser: it never builds a tree and never
/// reorders or hides characters. It maps a line of source text onto styled
/// runs covering exactly the same characters, so the buffer the user edits and
/// the buffer Flutter lays out are always the same length. That is what keeps
/// caret arithmetic and hit-testing honest.
///
/// Markers (`#`, `**`, backticks) are *dimmed*, not hidden. See the plan: true
/// hiding needs zero-width rendering, which breaks the caret.
library;

/// What a run of characters is, semantically. The theme maps these to styles.
enum TokenKind {
  text,
  marker, // syntax punctuation: #, **, `, [[, ]], >, -
  heading1,
  heading2,
  heading3,
  heading4,
  bold,
  italic,
  boldItalic,
  strikethrough,
  highlight,
  code,
  codeBlock,
  wikilink,
  linkText,
  linkUrl,
  tag,
  blockquote,
  listMarker,
  frontmatter,
  horizontalRule,
}

/// A styled run of characters, in absolute offsets into the full document.
class Token {
  const Token(this.start, this.end, this.kind);

  final int start;
  final int end;
  final TokenKind kind;

  int get length => end - start;

  @override
  bool operator ==(Object other) =>
      other is Token &&
      other.start == start &&
      other.end == end &&
      other.kind == kind;

  @override
  int get hashCode => Object.hash(start, end, kind);

  @override
  String toString() => 'Token($start-$end, $kind)';
}

/// Block-level context carried from one line to the next.
///
/// Tokenizing a line in isolation is wrong — whether a line is code depends on
/// whether an earlier line opened a fence. This is the minimum state needed to
/// resume mid-document, and it doubles as part of the per-line cache key.
class BlockContext {
  const BlockContext({this.inFence = false, this.inFrontmatter = false});

  final bool inFence;
  final bool inFrontmatter;

  @override
  bool operator ==(Object other) =>
      other is BlockContext &&
      other.inFence == inFence &&
      other.inFrontmatter == inFrontmatter;

  @override
  int get hashCode => Object.hash(inFence, inFrontmatter);
}

/// Result of tokenizing one line: its spans plus the context for the next line.
class LineResult {
  const LineResult(this.tokens, this.nextContext);

  final List<Token> tokens;
  final BlockContext nextContext;
}

// Inline patterns. Ordered by precedence — code wins over emphasis, so that
// `**not bold**` inside backticks stays literal.
final _codeSpan = RegExp(r'`([^`\n]+)`');
final _boldItalic = RegExp(r'\*\*\*([^*\n]+)\*\*\*');
final _bold = RegExp(r'\*\*([^*\n]+)\*\*|__([^_\n]+)__');
final _italic = RegExp(r'(?<![*\w])\*([^*\n]+)\*(?![*\w])|(?<![_\w])_([^_\n]+)_(?![_\w])');
final _strike = RegExp(r'~~([^~\n]+)~~');
final _highlight = RegExp(r'==([^=\n]+)==');
final _wikilink = RegExp(r'\[\[([^\[\]\n]+)\]\]');
final _mdLink = RegExp(r'\[([^\[\]\n]*)\]\(([^()\n]*)\)');
// A tag must start at a boundary and hold at least one non-numeric char, so
// `#1` and `#hex-in-a-url` don't light up as tags.
final _tag = RegExp(r'(?<![\w/])#([A-Za-zÀ-￿][\wÀ-￿/-]*)');

final _headingLine = RegExp(r'^(#{1,6})(\s+)(.*)$');
final _blockquoteLine = RegExp(r'^(\s*>+\s?)(.*)$');
final _bulletLine = RegExp(r'^(\s*)([-*+])(\s+)(.*)$');
final _orderedLine = RegExp(r'^(\s*)(\d+[.)])(\s+)(.*)$');
final _hrLine = RegExp(r'^\s*(?:[-*_])\s*(?:[-*_])\s*(?:[-*_])[-*_\s]*$');
final _fenceLine = RegExp(r'^\s*(?:```|~~~)');

/// Tokenizes a single line whose first character sits at [offset].
///
/// Returns runs in ascending order. Gaps are implicitly [TokenKind.text]; the
/// caller fills them, which keeps this function from emitting a token per
/// unstyled character.
LineResult tokenizeLine(String line, int offset, BlockContext ctx, int lineIndex) {
  // --- Frontmatter -----------------------------------------------------
  // Only a `---` on the very first line opens frontmatter; anywhere else it is
  // a horizontal rule.
  if (ctx.inFrontmatter) {
    final closing = line.trimRight() == '---';
    return LineResult(
      [Token(offset, offset + line.length, TokenKind.frontmatter)],
      BlockContext(inFrontmatter: !closing),
    );
  }
  if (lineIndex == 0 && line.trimRight() == '---') {
    return LineResult(
      [Token(offset, offset + line.length, TokenKind.frontmatter)],
      const BlockContext(inFrontmatter: true),
    );
  }

  // --- Fenced code -----------------------------------------------------
  if (_fenceLine.hasMatch(line)) {
    return LineResult(
      [Token(offset, offset + line.length, TokenKind.codeBlock)],
      BlockContext(inFence: !ctx.inFence),
    );
  }
  if (ctx.inFence) {
    return LineResult(
      [Token(offset, offset + line.length, TokenKind.codeBlock)],
      ctx,
    );
  }

  final tokens = <Token>[];

  // --- Block prefixes --------------------------------------------------
  // Each branch marks its prefix, then hands the remainder to the inline pass
  // with a base kind that the inline runs inherit.
  final heading = _headingLine.firstMatch(line);
  if (heading != null) {
    final level = heading.group(1)!.length;
    final prefixLen = heading.group(1)!.length + heading.group(2)!.length;
    tokens.add(Token(offset, offset + prefixLen, TokenKind.marker));
    _tokenizeInline(
      heading.group(3)!,
      offset + prefixLen,
      tokens,
      base: switch (level) {
        1 => TokenKind.heading1,
        2 => TokenKind.heading2,
        3 => TokenKind.heading3,
        _ => TokenKind.heading4,
      },
    );
    return LineResult(tokens, ctx);
  }

  if (_hrLine.hasMatch(line) && line.trim().length >= 3) {
    return LineResult(
      [Token(offset, offset + line.length, TokenKind.horizontalRule)],
      ctx,
    );
  }

  final quote = _blockquoteLine.firstMatch(line);
  if (quote != null) {
    final prefixLen = quote.group(1)!.length;
    tokens.add(Token(offset, offset + prefixLen, TokenKind.marker));
    _tokenizeInline(
      quote.group(2)!,
      offset + prefixLen,
      tokens,
      base: TokenKind.blockquote,
    );
    return LineResult(tokens, ctx);
  }

  final bullet = _bulletLine.firstMatch(line) ?? _orderedLine.firstMatch(line);
  if (bullet != null) {
    final indent = bullet.group(1)!.length;
    final markerLen = bullet.group(2)!.length + bullet.group(3)!.length;
    tokens.add(
      Token(offset + indent, offset + indent + markerLen, TokenKind.listMarker),
    );
    _tokenizeInline(bullet.group(4)!, offset + indent + markerLen, tokens);
    return LineResult(tokens, ctx);
  }

  _tokenizeInline(line, offset, tokens);
  return LineResult(tokens, ctx);
}

/// Scans [text] for inline constructs, appending runs to [out].
///
/// Single left-to-right pass. At each position the earliest-starting match
/// across all patterns wins, so patterns can't overlap and we never backtrack
/// over ground we've already emitted.
void _tokenizeInline(
  String text,
  int offset,
  List<Token> out, {
  TokenKind base = TokenKind.text,
}) {
  if (text.isEmpty) return;

  var cursor = 0;
  while (cursor < text.length) {
    _InlineMatch? best;

    for (final probe in _probes) {
      final m = probe.pattern.matchAsPrefix(text, cursor) ??
          probe.pattern.firstMatch(text.substring(cursor));
      if (m == null) continue;
      // firstMatch on a substring reports relative offsets; normalize.
      final start = m.input.length == text.length ? m.start : cursor + m.start;
      final end = m.input.length == text.length ? m.end : cursor + m.end;
      if (start < cursor) continue;
      if (best == null || start < best.start) {
        best = _InlineMatch(start, end, probe, m);
      }
    }

    if (best == null) break;

    if (best.start > cursor && base != TokenKind.text) {
      out.add(Token(offset + cursor, offset + best.start, base));
    }
    best.probe.emit(best, offset, out, base);
    cursor = best.end;
  }

  if (cursor < text.length && base != TokenKind.text) {
    out.add(Token(offset + cursor, offset + text.length, base));
  }
}

class _InlineMatch {
  _InlineMatch(this.start, this.end, this.probe, this.match);
  final int start;
  final int end;
  final _Probe probe;
  final Match match;
}

class _Probe {
  const _Probe(this.pattern, this.emit);
  final RegExp pattern;
  final void Function(_InlineMatch, int, List<Token>, TokenKind) emit;
}

/// Emits `<marker><content><marker>`, dimming the delimiters.
void Function(_InlineMatch, int, List<Token>, TokenKind) _delimited(
  int markerLen,
  TokenKind kind,
) {
  return (m, offset, out, base) {
    final s = offset + m.start;
    final e = offset + m.end;
    out.add(Token(s, s + markerLen, TokenKind.marker));
    out.add(Token(s + markerLen, e - markerLen, kind));
    out.add(Token(e - markerLen, e, TokenKind.marker));
  };
}

final _probes = <_Probe>[
  // Code first: it suppresses emphasis inside it.
  _Probe(_codeSpan, _delimited(1, TokenKind.code)),
  _Probe(_wikilink, _delimited(2, TokenKind.wikilink)),
  _Probe(_boldItalic, _delimited(3, TokenKind.boldItalic)),
  _Probe(_bold, _delimited(2, TokenKind.bold)),
  _Probe(_strike, _delimited(2, TokenKind.strikethrough)),
  _Probe(_highlight, _delimited(2, TokenKind.highlight)),
  _Probe(_italic, _delimited(1, TokenKind.italic)),
  _Probe(_mdLink, (m, offset, out, base) {
    // [text](url) — four marker runs around two content runs.
    final s = offset + m.start;
    final textLen = (m.match.group(1) ?? '').length;
    out.add(Token(s, s + 1, TokenKind.marker));
    out.add(Token(s + 1, s + 1 + textLen, TokenKind.linkText));
    out.add(Token(s + 1 + textLen, s + 3 + textLen, TokenKind.marker));
    out.add(Token(s + 3 + textLen, offset + m.end - 1, TokenKind.linkUrl));
    out.add(Token(offset + m.end - 1, offset + m.end, TokenKind.marker));
  }),
  _Probe(_tag, (m, offset, out, base) {
    out.add(Token(offset + m.start, offset + m.end, TokenKind.tag));
  }),
];
