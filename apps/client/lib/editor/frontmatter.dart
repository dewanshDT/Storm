/// Splitting a note into its YAML frontmatter and its body.
///
/// Mirrors `apps/server/src/frontmatter.rs` deliberately: the two must agree on
/// exactly which bytes are frontmatter, or a note will round-trip differently
/// depending on which side touched it last.
///
/// Nothing here rewrites YAML. The raw block is carried verbatim so that
/// [NoteParts.join] can reproduce the original file byte for byte — the editor
/// shows the body only, and everything it doesn't show must survive untouched.
library;

/// A note split into the frontmatter block and the body after it.
class NoteParts {
  const NoteParts({required this.frontmatter, required this.body});

  /// The whole block *including* both `---` fences and the trailing newline,
  /// or `''` when the note has none. Never reformatted.
  final String frontmatter;

  /// Everything after the closing fence.
  final String body;

  bool get hasFrontmatter => frontmatter.isNotEmpty;

  /// Reassembles the file. `join(split(x)) == x` for every input.
  String join() => frontmatter + body;

  /// Replaces the body, keeping the frontmatter exactly as it was.
  String withBody(String newBody) => frontmatter + newBody;
}

/// Splits [content] at the frontmatter fence.
///
/// A block only counts when `---` is the very first line, and only when a
/// closing `---` exists — an unterminated block is body text, not metadata.
/// Treating it as metadata would swallow the whole note.
NoteParts split(String content) {
  if (!_isFence(_firstLine(content))) {
    return NoteParts(frontmatter: '', body: content);
  }

  var offset = _firstLine(content).length;
  while (offset < content.length) {
    final line = _lineAt(content, offset);
    if (_isFence(line)) {
      final end = offset + line.length;
      return NoteParts(
        frontmatter: content.substring(0, end),
        body: content.substring(end),
      );
    }
    offset += line.length;
  }

  // No closing fence.
  return NoteParts(frontmatter: '', body: content);
}

String _firstLine(String s) => _lineAt(s, 0);

/// The line starting at [from], including its trailing newline if present.
String _lineAt(String s, int from) {
  final nl = s.indexOf('\n', from);
  return nl < 0 ? s.substring(from) : s.substring(from, nl + 1);
}

bool _isFence(String line) {
  final t = line.replaceAll('\r', '').replaceAll('\n', '').trimRight();
  return t == '---';
}

/// One key/value line from the frontmatter, for display.
class NoteProperty {
  const NoteProperty(this.key, this.value);

  final String key;
  final String value;

  /// Fields Storm owns. They're shown separately: `id` is the sync identity
  /// and editing it would orphan the note, and the timestamps are maintained
  /// by the server.
  static const managed = {'id', 'created', 'modified'};

  bool get isManaged => managed.contains(key);
}

/// Reads top-level `key: value` pairs for display, in file order.
///
/// Deliberately shallow — it renders what is there rather than interpreting
/// YAML. Nested structures are shown as their raw text so nothing is silently
/// misrepresented, and no value is ever written back from this.
List<NoteProperty> readProperties(String frontmatter) {
  final out = <NoteProperty>[];
  final inner = _innerLines(frontmatter);

  for (var i = 0; i < inner.length; i++) {
    final line = inner[i];
    if (line.trimLeft() != line) continue; // continuation of a nested value
    if (line.trim().isEmpty || line.trimLeft().startsWith('#')) continue;

    final colon = line.indexOf(':');
    if (colon <= 0) continue;

    final key = line.substring(0, colon).trim();
    var value = line.substring(colon + 1).trim();

    // A block list (`tags:` then `  - a`) reads as a flat, comma-joined value.
    if (value.isEmpty) {
      final items = <String>[];
      for (var j = i + 1; j < inner.length; j++) {
        final next = inner[j];
        if (next.trimLeft() == next && next.trim().isNotEmpty) break;
        final item = next.trim();
        if (item.startsWith('-')) items.add(item.substring(1).trim());
      }
      if (items.isNotEmpty) value = items.join(', ');
    }

    out.add(NoteProperty(key, _unquote(value)));
  }
  return out;
}

/// Convenience for the one property with real UI meaning.
List<String> readTags(String frontmatter) {
  for (final p in readProperties(frontmatter)) {
    if (p.key != 'tags') continue;
    return p.value
        .replaceAll('[', '')
        .replaceAll(']', '')
        .split(',')
        .map((t) => _unquote(t.trim()))
        .where((t) => t.isNotEmpty)
        .toList();
  }
  return const [];
}

List<String> _innerLines(String frontmatter) {
  final lines = frontmatter.split('\n');
  if (lines.length <= 2) return const [];
  // Drop the opening fence and everything from the closing fence on.
  final inner = <String>[];
  for (var i = 1; i < lines.length; i++) {
    if (_isFence(lines[i])) break;
    inner.add(lines[i]);
  }
  return inner;
}

String _unquote(String s) {
  if (s.length >= 2 &&
      ((s.startsWith('"') && s.endsWith('"')) ||
          (s.startsWith("'") && s.endsWith("'")))) {
    return s.substring(1, s.length - 1);
  }
  return s;
}
