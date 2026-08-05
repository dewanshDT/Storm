import '../api/models.dart';

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
