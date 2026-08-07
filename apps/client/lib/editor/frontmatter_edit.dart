/// Editing YAML frontmatter by splicing lines, never by serializing.
///
/// The companion to [frontmatter.dart], which only reads. Same rule as
/// `apps/server/src/frontmatter.rs`: a real vault carries arbitrary keys,
/// hand-chosen order, comments and inconsistent quoting, and running it
/// through a YAML emitter would rewrite every file on first touch. So every
/// operation here replaces a *range of lines* and passes every other byte
/// through untouched.
///
/// This is deliberately **not** a port of the server's `set_scalars`. That one
/// exists to stamp `id`, `created` and `modified`, and takes four shortcuts
/// that are fine for those and wrong for a user editing their own metadata:
///
///  * it replaces the key's single line, so pointing it at a block list
///    orphans the `- item` children and leaves invalid YAML;
///  * it rebuilds the block as `---\n…\n---\n`, normalising CRLF and fence
///    whitespace;
///  * it rewrites the whole line, destroying a trailing `# comment`;
///  * it does no quoting, so a value containing `: ` breaks the document.
///
/// Each of those is a case below, and each has a test.
library;

import 'frontmatter.dart' as fm;

/// How a property's value is written in the file.
///
/// The form is preserved across edits. A block list that becomes an inline
/// list on the first tag edit is a diff the user did not ask for.
enum PropertyForm {
  /// `key: value`
  scalar,

  /// `key: [a, b]`
  inlineList,

  /// `key:` followed by indented `- item` lines.
  blockList,

  /// `key:` followed by indented `sub: value` lines.
  nested,

  /// `key: |` or `key: >` followed by an indented block.
  blockScalar,
}

/// One property, located precisely enough to be rewritten in place.
class PropertySpan {
  const PropertySpan({
    required this.key,
    required this.firstLine,
    required this.lastLine,
    required this.form,
    required this.rawValue,
    required this.items,
    this.trailingComment,
  });

  final String key;

  /// Index of the `key:` line within the block's inner lines.
  final int firstLine;

  /// Index of the property's last line, inclusive. Equals [firstLine] for a
  /// scalar; covers the `- item` lines of a block list.
  final int lastLine;

  final PropertyForm form;

  /// The value exactly as written, quotes included. Empty for list and
  /// nested forms, whose content is on later lines.
  final String rawValue;

  /// List items, unquoted. Empty unless the form is a list.
  final List<String> items;

  /// A `# comment` at the end of the key's line, including the `#`.
  ///
  /// Preserved on write. The server's writer drops these.
  final String? trailingComment;

  /// Whether the typed editor may write this back.
  ///
  /// Nested maps and block scalars are shown read-only: representing them in
  /// a key/value row would mean guessing at a structure, and writing that
  /// guess back would destroy it. "Edit raw" is the honest answer for those.
  bool get isEditable =>
      form != PropertyForm.nested && form != PropertyForm.blockScalar;

  /// The unquoted scalar value, or the items joined for display.
  String get displayValue =>
      form == PropertyForm.scalar ? unquote(rawValue) : items.join(', ');
}

/// Every top-level property in [content], in file order.
List<PropertySpan> readSpans(String content) {
  final inner = _innerLines(fm.split(content).frontmatter);
  final out = <PropertySpan>[];

  var i = 0;
  while (i < inner.length) {
    final line = inner[i];

    // Blank lines, comments, and continuations of the property above.
    if (line.trim().isEmpty ||
        line.trimLeft().startsWith('#') ||
        line.trimLeft() != line) {
      i++;
      continue;
    }

    final colon = _keyColon(line);
    if (colon < 0) {
      i++;
      continue;
    }

    final key = line.substring(0, colon).trim();
    final after = line.substring(colon + 1);
    final (value, comment) = _splitTrailingComment(after);

    // `key: |` or `key: >` — everything indented below belongs to the value.
    if (value == '|' || value == '>' || value == '|-' || value == '>-') {
      out.add(
        PropertySpan(
          key: key,
          firstLine: i,
          lastLine: _lastIndentedFrom(inner, i),
          form: PropertyForm.blockScalar,
          rawValue: value,
          items: const [],
          trailingComment: comment,
        ),
      );
      i = _lastIndentedFrom(inner, i) + 1;
      continue;
    }

    if (value.isEmpty) {
      // `key:` with indented lines below is either a block list or a map.
      final last = _lastIndentedFrom(inner, i);
      if (last > i) {
        final children = inner.sublist(i + 1, last + 1);
        final isList = children
            .where((l) => l.trim().isNotEmpty)
            .every((l) => l.trimLeft().startsWith('-'));
        out.add(
          PropertySpan(
            key: key,
            firstLine: i,
            lastLine: last,
            form: isList ? PropertyForm.blockList : PropertyForm.nested,
            rawValue: '',
            items: isList
                ? [
                    for (final l in children)
                      if (l.trim().startsWith('-'))
                        unquote(l.trim().substring(1).trim()),
                  ]
                : const [],
            trailingComment: comment,
          ),
        );
        i = last + 1;
        continue;
      }
      // A bare `key:` with nothing under it is an empty scalar.
      out.add(
        PropertySpan(
          key: key,
          firstLine: i,
          lastLine: i,
          form: PropertyForm.scalar,
          rawValue: '',
          items: const [],
          trailingComment: comment,
        ),
      );
      i++;
      continue;
    }

    if (value.startsWith('[')) {
      out.add(
        PropertySpan(
          key: key,
          firstLine: i,
          lastLine: i,
          form: PropertyForm.inlineList,
          rawValue: value,
          items: _splitInlineList(value),
          trailingComment: comment,
        ),
      );
      i++;
      continue;
    }

    out.add(
      PropertySpan(
        key: key,
        firstLine: i,
        lastLine: i,
        form: PropertyForm.scalar,
        rawValue: value,
        items: const [],
        trailingComment: comment,
      ),
    );
    i++;
  }
  return out;
}

PropertySpan? findSpan(String content, String key) {
  for (final s in readSpans(content)) {
    if (s.key == key) return s;
  }
  return null;
}

// ---- writing -----------------------------------------------------------

/// Sets a scalar value, creating the key (and the block) if absent.
///
/// [raw] writes the value with no quoting, for callers that produce a valid
/// YAML scalar by construction — a checkbox emitting `true`, or a number
/// field emitting `-5`. Both would otherwise be quoted into strings by
/// [needsQuoting], which is the correct default for text a user typed.
String setScalar(String content, String key, String value, {bool raw = false}) {
  final existing = findSpan(content, key);
  if (existing == null) {
    return addProperty(content, key, value, alreadyEncoded: raw);
  }
  if (!existing.isEditable) return content;

  return _spliceLines(content, existing.firstLine, existing.lastLine, [
    raw
        ? '$key: $value${_comment(existing.trailingComment)}'
        : _scalarLine(key, value, existing.trailingComment),
  ]);
}

/// Sets a list value, **keeping the form it found**.
///
/// A list that has no entry yet is written inline, which is the compact form
/// and what Obsidian's own property editor produces.
String setList(String content, String key, List<String> items) {
  final existing = findSpan(content, key);
  if (existing == null) {
    return addProperty(content, key, _inlineList(items), alreadyEncoded: true);
  }
  if (!existing.isEditable) return content;

  final indent = existing.form == PropertyForm.blockList
      ? _indentOf(content, existing.firstLine + 1)
      : '  ';

  final replacement = existing.form == PropertyForm.blockList
      ? [
          '$key:${_comment(existing.trailingComment)}',
          for (final item in items) '$indent- ${_encode(item)}',
        ]
      : [
          '$key: ${_inlineList(items)}'
              '${_comment(existing.trailingComment)}',
        ];

  return _spliceLines(
    content,
    existing.firstLine,
    existing.lastLine,
    replacement,
  );
}

/// Appends a property at the bottom of the block.
///
/// The bottom, not the top: that is where the `+` sits in the UI, and where a
/// reader expects a just-added row to appear. The server inserts at the top
/// instead, because `id` belongs there.
String addProperty(
  String content,
  String key,
  String value, {
  bool alreadyEncoded = false,
}) {
  final withBlock = ensureBlock(content);
  final inner = _innerLines(fm.split(withBlock).frontmatter);
  final line = alreadyEncoded ? '$key: $value' : _scalarLine(key, value, null);
  return _spliceLines(withBlock, inner.length, inner.length - 1, [line]);
}

/// Removes a property and everything that belongs to it.
String removeProperty(String content, String key) {
  final existing = findSpan(content, key);
  if (existing == null) return content;
  return _spliceLines(content, existing.firstLine, existing.lastLine, const []);
}

/// Renames a key, leaving its value — and its form — exactly as they were.
String renameKey(String content, String from, String to) {
  final existing = findSpan(content, from);
  if (existing == null || from == to) return content;

  final lines = _innerLines(fm.split(content).frontmatter);
  final first = lines[existing.firstLine];
  final colon = _keyColon(first);
  final rebuilt = '$to${first.substring(colon)}';

  return _spliceLines(content, existing.firstLine, existing.firstLine, [
    rebuilt,
  ]);
}

/// Gives [content] a frontmatter block if it has none, changing nothing else.
String ensureBlock(String content) {
  if (fm.split(content).hasFrontmatter) return content;
  final eol = detectEol(content);
  final separator = content.isEmpty || content.startsWith('\n') ? '' : eol;
  return '---$eol---$eol$separator$content';
}

// ---- encoding ----------------------------------------------------------

/// Values YAML would read as something other than a string.
final _boolish = RegExp(
  r'^(true|false|yes|no|on|off|null|~)$',
  caseSensitive: false,
);

/// Whether [value] has to be quoted to survive a YAML round trip.
///
/// The server's writer does none of this, which is safe only because it only
/// ever writes UUIDs and RFC3339 timestamps. A user's value can be anything.
///
/// [inList] tightens the rule for an inline-list item, where a comma or a
/// closing bracket would end the item rather than sit inside it. A comma is
/// perfectly fine in a scalar, so the context has to be passed in.
bool needsQuoting(String value, {bool inList = false}) {
  if (value.isEmpty) return false; // `key:` is a valid empty value
  if (value != value.trim()) return true; // leading/trailing space
  if (value.contains(': ') || value.endsWith(':')) return true;
  if (value.contains(' #')) return true; // would start a comment
  if (value.contains('\n')) return true;
  if (_boolish.hasMatch(value)) return true;
  if (inList && (value.contains(',') || value.contains(']'))) return true;
  return '-?:,[]{}#&*!|>\'"%@`'.contains(value[0]);
}

/// Encodes a value for the right-hand side of `key:`.
String _encode(String value, {bool inList = false}) {
  if (!needsQuoting(value, inList: inList)) return value;
  // Double quotes, with the two characters that matter inside them escaped.
  final escaped = value.replaceAll(r'\', r'\\').replaceAll('"', r'\"');
  return '"$escaped"';
}

/// Strips one layer of matching quotes. Shared with the reader's behaviour.
String unquote(String s) {
  if (s.length >= 2 &&
      ((s.startsWith('"') && s.endsWith('"')) ||
          (s.startsWith("'") && s.endsWith("'")))) {
    final inner = s.substring(1, s.length - 1);
    return s.startsWith('"')
        ? inner.replaceAll(r'\"', '"').replaceAll(r'\\', r'\')
        : inner;
  }
  return s;
}

String _inlineList(List<String> items) =>
    '[${items.map((i) => _encode(i, inList: true)).join(', ')}]';

String _scalarLine(String key, String value, String? comment) {
  final encoded = _encode(value);
  final body = encoded.isEmpty ? '$key:' : '$key: $encoded';
  return '$body${_comment(comment)}';
}

String _comment(String? comment) => comment == null ? '' : ' $comment';

// ---- line surgery ------------------------------------------------------

/// The line ending the file uses, so a write never changes it.
///
/// The server's writer normalises everything inside the block to `\n`. This
/// one runs on ordinary edits, so a CRLF note must stay CRLF or the first
/// property change would show up as a whole-file diff.
String detectEol(String content) => content.contains('\r\n') ? '\r\n' : '\n';

/// Replaces inner lines `[from, to]` with [replacement], leaving every other
/// byte of the file — fences, body, comments, indentation — untouched.
///
/// `to < from` inserts at `from` without removing anything.
String _spliceLines(
  String content,
  int from,
  int to,
  List<String> replacement,
) {
  final parts = fm.split(content);
  if (!parts.hasFrontmatter) return content;

  final eol = detectEol(content);
  final block = parts.frontmatter;

  // Rebuild only the inner region; the fences are copied verbatim so their
  // exact bytes (trailing spaces, CRLF) survive.
  final openEnd = _lineEnd(block, 0);
  final open = block.substring(0, openEnd);

  final inner = _innerLines(block);
  final closeStart = _closeFenceStart(block);
  final close = block.substring(closeStart);

  final kept = <String>[
    ...inner.take(from),
    ...replacement,
    if (to >= from) ...inner.skip(to + 1) else ...inner.skip(from),
  ];

  final rebuilt = StringBuffer(open);
  for (final line in kept) {
    rebuilt
      ..write(line)
      ..write(eol);
  }
  rebuilt.write(close);

  return rebuilt.toString() + parts.body;
}

/// Inner lines with their line endings stripped, in order.
List<String> _innerLines(String block) {
  if (block.isEmpty) return const [];
  final out = <String>[];
  var offset = _lineEnd(block, 0);
  final closeStart = _closeFenceStart(block);
  while (offset < closeStart) {
    final end = _lineEnd(block, offset);
    out.add(block.substring(offset, end).replaceAll('\r', '').trimRight());
    offset = end;
  }
  return out;
}

/// Offset just past the newline of the line starting at [from].
int _lineEnd(String s, int from) {
  final nl = s.indexOf('\n', from);
  return nl < 0 ? s.length : nl + 1;
}

/// Offset where the closing fence line begins.
int _closeFenceStart(String block) {
  var offset = _lineEnd(block, 0);
  var last = offset;
  while (offset < block.length) {
    final end = _lineEnd(block, offset);
    final line = block.substring(offset, end).replaceAll('\r', '').trimRight();
    if (line.replaceAll('\n', '') == '---') return offset;
    last = end;
    offset = end;
  }
  return last;
}

/// The indentation of an inner line, defaulting to two spaces.
String _indentOf(String content, int lineIndex) {
  final inner = _innerLines(fm.split(content).frontmatter);
  if (lineIndex < 0 || lineIndex >= inner.length) return '  ';
  final line = inner[lineIndex];
  final trimmed = line.trimLeft();
  if (trimmed.isEmpty) return '  ';
  final indent = line.substring(0, line.length - trimmed.length);
  return indent.isEmpty ? '  ' : indent;
}

/// Index of the last line belonging to the property starting at [start].
int _lastIndentedFrom(List<String> inner, int start) {
  var last = start;
  for (var i = start + 1; i < inner.length; i++) {
    final line = inner[i];
    if (line.trim().isEmpty) continue; // a blank line does not end the value
    if (line.trimLeft() == line) break; // back at column zero
    last = i;
  }
  return last;
}

/// The position of the `:` that separates a top-level key from its value, or
/// `-1` when the line is not a key.
///
/// A quoted key may contain a colon, so the scan skips quoted regions.
int _keyColon(String line) {
  var quote = '';
  for (var i = 0; i < line.length; i++) {
    final c = line[i];
    if (quote.isNotEmpty) {
      if (c == quote) quote = '';
      continue;
    }
    if (c == '"' || c == "'") {
      quote = c;
      continue;
    }
    if (c == ':') return i > 0 ? i : -1;
  }
  return -1;
}

/// Splits `value # comment` into its two parts.
///
/// Only a `#` preceded by whitespace starts a comment; `#tag` inside a value
/// is part of the value.
(String, String?) _splitTrailingComment(String after) {
  var quote = '';
  for (var i = 0; i < after.length; i++) {
    final c = after[i];
    if (quote.isNotEmpty) {
      if (c == quote) quote = '';
      continue;
    }
    if (c == '"' || c == "'") {
      quote = c;
      continue;
    }
    if (c == '#' && i > 0 && (after[i - 1] == ' ' || after[i - 1] == '\t')) {
      return (after.substring(0, i).trim(), after.substring(i).trim());
    }
  }
  return (after.trim(), null);
}

/// Splits `[a, "b, c"]` respecting quotes.
List<String> _splitInlineList(String raw) {
  final inner = raw.trim();
  final body = inner.startsWith('[')
      ? inner.substring(1, inner.endsWith(']') ? inner.length - 1 : null)
      : inner;

  final out = <String>[];
  final buffer = StringBuffer();
  var quote = '';
  for (var i = 0; i < body.length; i++) {
    final c = body[i];
    if (quote.isNotEmpty) {
      if (c == quote) quote = '';
      buffer.write(c);
      continue;
    }
    if (c == '"' || c == "'") {
      quote = c;
      buffer.write(c);
      continue;
    }
    if (c == ',') {
      out.add(unquote(buffer.toString().trim()));
      buffer.clear();
      continue;
    }
    buffer.write(c);
  }
  final last = unquote(buffer.toString().trim());
  if (last.isNotEmpty) out.add(last);
  return out.where((s) => s.isNotEmpty).toList();
}
