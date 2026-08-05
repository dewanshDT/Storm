import 'package:flutter/material.dart';

import 'markdown_tokenizer.dart';

/// Maps [TokenKind] to concrete text styles.
///
/// Syntax markers are rendered at low opacity rather than hidden — the buffer
/// must keep the same character count the user typed, so we recede them
/// visually instead of removing them.
class MarkdownTheme {
  MarkdownTheme({
    required this.base,
    required this.markerColor,
    required this.accent,
    required this.codeColor,
    required this.codeBackground,
    required this.mutedColor,
    required this.highlightBackground,
  }) {
    _styles = _build();
  }

  factory MarkdownTheme.light(TextStyle base) => MarkdownTheme(
    base: base,
    markerColor: const Color(0xFFB0B6C0),
    accent: const Color(0xFF5B6ABF),
    codeColor: const Color(0xFFB4426B),
    codeBackground: const Color(0x0F000000),
    mutedColor: const Color(0xFF6B7280),
    highlightBackground: const Color(0x40FFD54F),
  );

  factory MarkdownTheme.dark(TextStyle base) => MarkdownTheme(
    base: base,
    markerColor: const Color(0xFF5A6472),
    accent: const Color(0xFF8FA0F0),
    codeColor: const Color(0xFFE79AB8),
    codeBackground: const Color(0x1AFFFFFF),
    mutedColor: const Color(0xFF9AA3B0),
    highlightBackground: const Color(0x40FFC107),
  );

  final TextStyle base;
  final Color markerColor;
  final Color accent;
  final Color codeColor;
  final Color codeBackground;
  final Color mutedColor;
  final Color highlightBackground;

  late final Map<TokenKind, TextStyle> _styles;

  TextStyle styleFor(TokenKind kind) => _styles[kind] ?? base;

  /// Syntax on a line the caret isn't on.
  ///
  /// Not `fontSize: 0` — Flutter still lays out a glyph box, and a hard zero
  /// can collapse the line's height. A hair above zero, fully transparent, is
  /// invisible while keeping the character present so caret offsets stay
  /// correct.
  ///
  /// Built once, not per access: this is read for every hidden run on every
  /// line, and as a getter calling `copyWith` it allocated thousands of
  /// TextStyles per rebuild and cost ~6x on a long document.
  late final TextStyle hiddenMarker = base.copyWith(
    fontSize: 0.01,
    color: const Color(0x00000000),
    letterSpacing: 0,
    height: base.height,
  );

  // Value equality is load-bearing, not a nicety. The editor assigns a theme
  // on every build; without this, each assignment looks like a change, and a
  // controller that invalidates on "change" ends up churning every frame.
  @override
  bool operator ==(Object other) =>
      other is MarkdownTheme &&
      other.base == base &&
      other.markerColor == markerColor &&
      other.accent == accent &&
      other.codeColor == codeColor &&
      other.codeBackground == codeBackground &&
      other.mutedColor == mutedColor &&
      other.highlightBackground == highlightBackground;

  @override
  int get hashCode => Object.hash(
    base,
    markerColor,
    accent,
    codeColor,
    codeBackground,
    mutedColor,
    highlightBackground,
  );

  Map<TokenKind, TextStyle> _build() {
    // Headings scale down by level. Kept modest — an h1 three times body size
    // makes the caret jump uncomfortably between lines while editing.
    TextStyle heading(double scale) => base.copyWith(
      fontSize: (base.fontSize ?? 16) * scale,
      fontWeight: FontWeight.w700,
      height: 1.35,
    );

    final mono = base.copyWith(
      fontFamily: 'monospace',
      fontFamilyFallback: const ['Menlo', 'Consolas', 'DejaVu Sans Mono'],
    );

    return {
      TokenKind.text: base,
      TokenKind.marker: base.copyWith(color: markerColor),
      TokenKind.heading1: heading(1.6),
      TokenKind.heading2: heading(1.4),
      TokenKind.heading3: heading(1.2),
      TokenKind.heading4: heading(1.1),
      TokenKind.bold: base.copyWith(fontWeight: FontWeight.w700),
      TokenKind.italic: base.copyWith(fontStyle: FontStyle.italic),
      TokenKind.boldItalic: base.copyWith(
        fontWeight: FontWeight.w700,
        fontStyle: FontStyle.italic,
      ),
      TokenKind.strikethrough: base.copyWith(
        decoration: TextDecoration.lineThrough,
        color: mutedColor,
      ),
      TokenKind.highlight: base.copyWith(backgroundColor: highlightBackground),
      TokenKind.code: mono.copyWith(
        color: codeColor,
        backgroundColor: codeBackground,
      ),
      TokenKind.codeBlock: mono.copyWith(
        color: mutedColor,
        backgroundColor: codeBackground,
      ),
      TokenKind.wikilink: base.copyWith(
        color: accent,
        fontWeight: FontWeight.w500,
      ),
      TokenKind.linkText: base.copyWith(color: accent),
      TokenKind.linkUrl: base.copyWith(
        color: mutedColor,
        decoration: TextDecoration.underline,
        decorationColor: mutedColor,
      ),
      TokenKind.tag: base.copyWith(
        color: accent,
        backgroundColor: accent.withValues(alpha: 0.10),
        fontWeight: FontWeight.w500,
      ),
      TokenKind.blockquote: base.copyWith(
        color: mutedColor,
        fontStyle: FontStyle.italic,
      ),
      TokenKind.listMarker: base.copyWith(
        color: accent,
        fontWeight: FontWeight.w700,
      ),
      TokenKind.frontmatter: mono.copyWith(color: mutedColor, fontSize: 13),
      TokenKind.horizontalRule: base.copyWith(color: markerColor),
    };
  }
}
