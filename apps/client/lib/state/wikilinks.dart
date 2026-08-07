import '../api/models.dart';
import 'vault_config.dart';

/// Resolves `[[target]]` to a note, the way a vault of files implies.
///
/// The server indexes links for *backlinks*, but following one forwards is a
/// client question — it depends on what the user can see. The rules follow
/// Obsidian, in order, because that is what the vault was written against:
///
///  1. exact path (`Daily/2026-08-05.md`, or without the extension),
///  2. exact filename without extension,
///  3. exact title from frontmatter,
///  4. the same three again, case-insensitively.
///
/// Returns null when nothing matches — an unresolved link is a normal state in
/// a vault, not an error, and the caller says so rather than inventing a note.
NoteMeta? resolveWikilink(List<NoteMeta> notes, String target) {
  final want = target.trim();
  if (want.isEmpty) return null;

  // A `[[Note#heading]]` or `[[Note|alias]]` still points at Note.
  final name = want.split('#').first.split('|').first.trim();
  if (name.isEmpty) return null;

  final withoutExt = name.endsWith('.md')
      ? name.substring(0, name.length - 3)
      : name;

  for (final compare in [_exact, _caseless]) {
    for (final note in notes) {
      // Storm's own config note is not a link target.
      if (isVaultConfigPath(note.path)) continue;
      final path = note.path;
      final pathNoExt = path.endsWith('.md')
          ? path.substring(0, path.length - 3)
          : path;
      final file = pathNoExt.split('/').last;

      if (compare(path, name) ||
          compare(pathNoExt, withoutExt) ||
          compare(file, withoutExt) ||
          (note.title.isNotEmpty && compare(note.title, name))) {
        return note;
      }
    }
  }
  return null;
}

bool _exact(String a, String b) => a == b;
bool _caseless(String a, String b) => a.toLowerCase() == b.toLowerCase();

/// Notes worth offering for a half-typed `[[query`.
///
/// Ranked so the thing you meant is first: names that *start* with what you
/// typed beat names that merely contain it, and a shorter name beats a longer
/// one, because a query is a prefix of the short name more often than of the
/// long one. Ties break alphabetically so the order never jitters between
/// keystrokes.
///
/// An empty query lists recent notes rather than nothing — opening `[[` on a
/// phone should show you somewhere to go, not an empty box.
List<NoteMeta> suggestWikilinks(
  List<NoteMeta> notes,
  String query, {
  int limit = 8,
}) {
  final want = query.trim().toLowerCase();
  // Never suggest Storm's own config note.
  final visible = notes.where((n) => !isVaultConfigPath(n.path)).toList();
  if (want.isEmpty) {
    final recent = [...visible]
      ..sort((a, b) => b.modified.compareTo(a.modified));
    return recent.take(limit).toList();
  }

  final scored = <(int, String, NoteMeta)>[];
  for (final note in visible) {
    final name = wikilinkDisplayName(note);
    final haystack = name.toLowerCase();
    final inPath = note.path.toLowerCase();

    final rank = haystack.startsWith(want)
        ? 0
        : haystack.contains(want)
        ? 1
        : inPath.contains(want)
        ? 2
        : -1;
    if (rank < 0) continue;
    scored.add((rank, name, note));
  }

  scored.sort((a, b) {
    if (a.$1 != b.$1) return a.$1.compareTo(b.$1);
    if (a.$2.length != b.$2.length) return a.$2.length.compareTo(b.$2.length);
    return a.$2.toLowerCase().compareTo(b.$2.toLowerCase());
  });
  return [for (final s in scored.take(limit)) s.$3];
}

/// What to call a note in the suggestion list.
///
/// The same name the suggestions are *matched* against, so what you see is
/// what you typed towards — a list that matches on one string and displays
/// another looks broken even when it is right.
String wikilinkDisplayName(NoteMeta note) {
  if (note.title.isNotEmpty) return note.title;
  final path = note.path.endsWith('.md')
      ? note.path.substring(0, note.path.length - 3)
      : note.path;
  return path.split('/').last;
}

/// What a completed `[[…]]` should contain for [note].
///
/// The bare name when it is unambiguous, the full path when two notes share a
/// name — writing an ambiguous link would resolve to whichever the resolver
/// happened to see first.
String wikilinkTargetFor(List<NoteMeta> notes, NoteMeta note) {
  final path = note.path.endsWith('.md')
      ? note.path.substring(0, note.path.length - 3)
      : note.path;
  final name = path.split('/').last;

  var sharing = 0;
  for (final other in notes) {
    final otherPath = other.path.endsWith('.md')
        ? other.path.substring(0, other.path.length - 3)
        : other.path;
    if (otherPath.split('/').last.toLowerCase() == name.toLowerCase()) {
      sharing++;
    }
  }
  return sharing > 1 ? path : name;
}
