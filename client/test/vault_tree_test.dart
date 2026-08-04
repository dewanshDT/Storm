import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/models.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/vault_screen.dart';
import 'package:storm/ui/vault_tree.dart';

NoteMeta note(String path) => NoteMeta(
      id: path,
      path: path,
      title: path,
      version: 1,
      modified: '',
      size: 0,
    );

void main() {
  group('tree building', () {
    test('nests notes under their folders', () {
      final root = TreeNode.build([
        note('Welcome.md'),
        note('Daily/2026-08-05.md'),
        note('Projects/Storm/Design.md'),
      ]);

      expect(root.children.keys, containsAll(['Welcome.md', 'Daily', 'Projects']));
      expect(root.children['Daily']!.children.keys, ['2026-08-05.md']);
      expect(
        root.children['Projects']!.children['Storm']!.children.keys,
        ['Design.md'],
      );
    });

    test('folders sort before notes, each alphabetically', () {
      final root = TreeNode.build([
        note('zebra.md'),
        note('Alpha.md'),
        note('Zoo/a.md'),
        note('Beta/a.md'),
      ]);

      expect(
        root.sortedChildren.map((n) => n.name).toList(),
        ['Beta', 'Zoo', 'Alpha.md', 'zebra.md'],
      );
    });

    test('an empty vault yields an empty tree', () {
      expect(TreeNode.build([]).children, isEmpty);
    });

    test('deeply nested paths keep their full structure', () {
      final root = TreeNode.build([note('a/b/c/d/e.md')]);
      var cursor = root;
      for (final segment in ['a', 'b', 'c', 'd']) {
        cursor = cursor.children[segment]!;
        expect(cursor.isFolder, isTrue);
      }
      expect(cursor.children['e.md']!.isFolder, isFalse);
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

  group('path validation mirrors the server rules', () {
    test('accepts ordinary paths', () {
      expect(validatePath('Note.md'), isNull);
      expect(validatePath('Daily/2026/08/05.md'), isNull);
      expect(validatePath('A note with spaces.md'), isNull);
      expect(validatePath('日本語/ノート.md'), isNull);
    });

    test('rejects what the server would reject', () {
      // Caught here so the user gets a clear message instead of a 400.
      expect(validatePath(''), isNotNull);
      expect(validatePath('/absolute.md'), isNotNull);
      expect(validatePath('../escape.md'), isNotNull);
      expect(validatePath('Notes/../../escape.md'), isNotNull);
      expect(validatePath('.hidden/x.md'), isNotNull);
      expect(validatePath('Notes/.obsidian/x.md'), isNotNull);
      expect(validatePath('Notes//double.md'), isNotNull);
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
