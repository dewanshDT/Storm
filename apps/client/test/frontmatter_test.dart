import 'package:flutter_test/flutter_test.dart';

import 'package:storm/editor/frontmatter.dart';

/// The frontmatter split.
///
/// The editor shows only the body, so everything it does *not* show has to
/// survive a round trip untouched. `join(split(x)) == x` is the property that
/// makes hiding the frontmatter safe; if it ever fails, saving a note silently
/// rewrites metadata the user never touched.
void main() {
  group('round-trips byte for byte', () {
    final corpus = <String, String>{
      'empty': '',
      'no frontmatter': '# Heading\n\nbody\n',
      'simple': '---\nid: abc\n---\n\n# Heading\n',
      'no trailing newline': '---\nid: abc\n---\nbody',
      'no body at all': '---\nid: abc\n---\n',
      'messy real-world':
          '---\n'
          'aliases: [storm, sync-design]\n'
          'cssclass: wide-page\n'
          '# a comment I wrote by hand\n'
          'publish: false\n'
          'tags: [homelab, project]\n'
          '---\n\n# Storm Design\n\nbody\n',
      'block list tags': '---\ntags:\n  - someday\n  - maybe\n---\n\nbody\n',
      'nested values': '---\nmeta:\n  a: 1\n  b: 2\n---\n\nbody\n',
      'body contains a horizontal rule': '---\nid: x\n---\n\na\n\n---\n\nb\n',
      'unterminated block': '---\nid: abc\n\n# Heading\n',
      'fence not on line one': '\n---\nid: abc\n---\n',
      'body starting with ---': '---\nid: x\n---\n---\nnot frontmatter\n',
      'crlf': '---\r\nid: abc\r\n---\r\nbody\r\n',
      'unicode': '---\ntitle: 日本語\n---\n\n# 見出し\n\n本文 🎉\n',
      'blank line in block': '---\na: 1\n\nb: 2\n---\nbody\n',
    };

    corpus.forEach((name, source) {
      test(name, () {
        final parts = split(source);
        expect(
          parts.join(),
          equals(source),
          reason: 'split/join must be lossless for: $name',
        );
      });
    });

    test('replacing the body keeps the frontmatter exactly', () {
      const src = '---\nid: abc\ntags: [a]\n# comment\n---\n\nold body\n';
      final parts = split(src);
      final out = parts.withBody('\nnew body\n');

      expect(out, '---\nid: abc\ntags: [a]\n# comment\n---\n\nnew body\n');
      expect(out, contains('# comment'), reason: 'comments must survive');
    });
  });

  group('splits at the right place', () {
    test('an unterminated block is body, not metadata', () {
      // Otherwise the editor would hide the entire note.
      final parts = split('---\nid: abc\n\n# Heading\n');
      expect(parts.hasFrontmatter, isFalse);
      expect(parts.body, contains('# Heading'));
    });

    test('a horizontal rule in the body is not a closing fence', () {
      final parts = split('---\nid: x\n---\n\na\n\n---\n\nb\n');
      expect(parts.frontmatter, '---\nid: x\n---\n');
      expect(parts.body, '\na\n\n---\n\nb\n');
    });

    test('a fence must be the very first line', () {
      expect(split('\n---\nid: abc\n---\n').hasFrontmatter, isFalse);
    });

    test('a note with no frontmatter is all body', () {
      const src = '# Just a note\n';
      final parts = split(src);
      expect(parts.hasFrontmatter, isFalse);
      expect(parts.body, src);
    });
  });

  group('reads properties for display', () {
    const messy =
        '---\n'
        'id: abc-123\n'
        'aliases: [storm, design]\n'
        'cssclass: wide-page\n'
        '# a comment\n'
        'publish: false\n'
        'tags: [homelab, project]\n'
        '---\n\nbody\n';

    test('in file order, comments skipped', () {
      final props = readProperties(split(messy).frontmatter);
      expect(props.map((p) => p.key).toList(), [
        'id',
        'aliases',
        'cssclass',
        'publish',
        'tags',
      ]);
    });

    test('marks the fields Storm owns', () {
      final props = readProperties(split(messy).frontmatter);
      final managed = props.where((p) => p.isManaged).map((p) => p.key);
      expect(managed, ['id'], reason: 'id is the sync identity');
    });

    test('flattens a block list', () {
      const src = '---\ntags:\n  - someday\n  - maybe\n---\nbody\n';
      final props = readProperties(split(src).frontmatter);
      expect(props.single.key, 'tags');
      expect(props.single.value, 'someday, maybe');
    });

    test('reads tags in both inline and block form', () {
      expect(readTags(split(messy).frontmatter), ['homelab', 'project']);
      expect(readTags(split('---\ntags:\n  - a\n  - b\n---\n').frontmatter), [
        'a',
        'b',
      ]);
      expect(readTags(split('---\nid: x\n---\n').frontmatter), isEmpty);
    });

    test('strips quotes', () {
      final props = readProperties(
        split('---\ntitle: "My Note: a study"\n---\n').frontmatter,
      );
      expect(props.single.value, 'My Note: a study');
    });

    test('a note without frontmatter has no properties', () {
      expect(readProperties(split('# Note\n').frontmatter), isEmpty);
    });
  });
}
