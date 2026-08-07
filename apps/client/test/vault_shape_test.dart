import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/models.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/browse_screen.dart';

NoteMeta note(String path) => NoteMeta(
  id: path,
  path: path,
  title: path,
  version: 1,
  modified: '',
  size: 0,
);

/// Deriving the vault's shape from note paths.
///
/// The recursive `TreeNode` this used to test is gone with the drawer shell:
/// the browser renders one level at a time, so the hierarchy question is now
/// "what is directly inside this folder". The cases are the same ones — they
/// are properties of a vault, not of a widget.
void main() {
  group('folder contents', () {
    test('nests notes under their folders', () {
      final notes = [
        note('Welcome.md'),
        note('Daily/2026-08-05.md'),
        note('Projects/Storm/Design.md'),
      ];

      expect(
        childrenOfFolder(notes, '').map((e) => e.name),
        containsAll(['Welcome', 'Daily', 'Projects']),
      );
      expect(childrenOfFolder(notes, 'Daily').map((e) => e.name), [
        '2026-08-05',
      ]);
      expect(childrenOfFolder(notes, 'Projects').map((e) => e.name), ['Storm']);
      expect(childrenOfFolder(notes, 'Projects/Storm').map((e) => e.name), [
        'Design',
      ]);
    });

    test('folders sort before notes, each alphabetically', () {
      final root = childrenOfFolder([
        note('zebra.md'),
        note('Alpha.md'),
        note('Zoo/a.md'),
        note('Beta/a.md'),
      ], '');

      expect(root.map((e) => e.name).toList(), [
        'Beta',
        'Zoo',
        'Alpha',
        'zebra',
      ]);
      expect(root.take(2).every((e) => e.isFolder), isTrue);
      expect(root.skip(2).any((e) => e.isFolder), isFalse);
    });

    test('an empty vault yields nothing', () {
      expect(childrenOfFolder([], ''), isEmpty);
    });

    test('deeply nested paths keep their full structure', () {
      final notes = [note('a/b/c/d/e.md')];
      for (final (folder, child) in [
        ('', 'a'),
        ('a', 'b'),
        ('a/b', 'c'),
        ('a/b/c', 'd'),
      ]) {
        final entries = childrenOfFolder(notes, folder);
        expect(entries.map((e) => e.name), [child]);
        expect(entries.single.isFolder, isTrue);
      }
      final leaf = childrenOfFolder(notes, 'a/b/c/d');
      expect(leaf.single.name, 'e');
      expect(leaf.single.isFolder, isFalse);
    });

    test('a folder name that prefixes another is not confused with it', () {
      // "Project" must not swallow "Projects".
      final notes = [note('Project/a.md'), note('Projects/b.md')];
      expect(childrenOfFolder(notes, 'Project').map((e) => e.name), ['a']);
      expect(childrenOfFolder(notes, 'Projects').map((e) => e.name), ['b']);
    });
  });

  group('note metadata', () {
    test('splits folder and file name', () {
      expect(note('Daily/2026-08-05.md').folder, 'Daily');
      expect(note('Daily/2026-08-05.md').fileName, '2026-08-05.md');
      expect(note('Root.md').folder, '');
      expect(note('Root.md').fileName, 'Root.md');
    });
  });

  group('url normalisation', () {
    test('adds a scheme and strips trailing slashes', () {
      expect(normalizeUrl('192.168.1.20:8484'), 'http://192.168.1.20:8484');
      expect(normalizeUrl('http://host:8484/'), 'http://host:8484');
      expect(normalizeUrl('https://host///'), 'https://host');
      expect(normalizeUrl('  host:1  '), 'http://host:1');
      expect(normalizeUrl(''), '');
    });
  });
}
