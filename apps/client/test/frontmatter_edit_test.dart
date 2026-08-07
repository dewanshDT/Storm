import 'package:flutter_test/flutter_test.dart';

import 'package:storm/editor/frontmatter.dart' as fm;
import 'package:storm/editor/frontmatter_edit.dart';

/// Editing frontmatter without serializing it.
///
/// The property every test here defends: **an edit changes the lines it
/// targets and not one byte more.** A vault carries hand-chosen key order,
/// comments, quoting habits and indentation, and the moment a property editor
/// starts reformatting those, every note it touches produces a diff the user
/// never asked for.
///
/// The server's `set_scalars` cannot be reused for this. It replaces a key's
/// single line, which is right for stamping `id` and wrong for a block list —
/// it would leave the `- item` children orphaned under a scalar. Several
/// cases below are exactly the ones it would get wrong.
void main() {
  /// Everything except the lines belonging to [key] must be unchanged.
  ///
  /// Compares line-wise so a failure points at the line that moved rather
  /// than at a whole-file diff.
  void expectOnlyTouched(String before, String after, String key) {
    final b = fm.split(before);
    final a = fm.split(after);
    expect(a.body, b.body, reason: 'the body must never move');

    List<String> others(String block) => block
        .split('\n')
        .where((l) => !l.trimLeft().startsWith('$key:'))
        .where((l) => !l.trimLeft().startsWith('- '))
        .toList();

    expect(
      others(a.frontmatter),
      others(b.frontmatter),
      reason: 'lines other than "$key" must be byte-identical',
    );
  }

  group('scalars', () {
    const src =
        '---\n'
        'title: Storm\n'
        '# a comment I wrote by hand\n'
        'status: draft\n'
        '---\n\n# Heading\n\nbody\n';

    test('replacing a value keeps the key in place', () {
      final out = setScalar(src, 'status', 'done');
      expect(out, contains('status: done'));
      expect(
        out.indexOf('title:') < out.indexOf('status:'),
        isTrue,
        reason: 'key order must not change',
      );
      expectOnlyTouched(src, out, 'status');
    });

    test('a hand-written comment line survives', () {
      final out = setScalar(src, 'status', 'done');
      expect(out, contains('# a comment I wrote by hand'));
    });

    test('a trailing comment on the edited line survives', () {
      // The server's writer rebuilds the whole line and destroys this.
      const withComment = '---\nid: abc # assigned at import\n---\nbody\n';
      final out = setScalar(withComment, 'id', 'xyz');
      expect(out, '---\nid: xyz # assigned at import\n---\nbody\n');
    });

    test('writing the same value back is byte-identical', () {
      // Otherwise simply opening a note would dirty it.
      expect(setScalar(src, 'status', 'draft'), src);
    });

    test('a missing key is appended at the bottom of the block', () {
      final out = setScalar(src, 'priority', 'high');
      final lines = fm.split(out).frontmatter.split('\n');
      expect(lines[lines.length - 3], 'priority: high');
      expect(out, contains('status: draft'));
    });

    test('a note with no frontmatter gets a block', () {
      final out = setScalar('# Heading\n\nbody\n', 'status', 'new');
      expect(out, '---\nstatus: new\n---\n\n# Heading\n\nbody\n');
    });

    test('an empty note gets a block', () {
      expect(setScalar('', 'a', 'b'), '---\na: b\n---\n');
    });
  });

  group('quoting', () {
    String write(String value) =>
        setScalar('---\nk: x\n---\nbody\n', 'k', value);

    test('values YAML would misread are quoted', () {
      // Each of these written bare either breaks the document or changes type.
      expect(write('My Note: a study'), contains('k: "My Note: a study"'));
      expect(write('#hash'), contains('k: "#hash"'));
      expect(write('- dash'), contains('k: "- dash"'));
      expect(write(' padded '), contains('k: " padded "'));
      expect(write('true'), contains('k: "true"'));
      expect(write('null'), contains('k: "null"'));
      expect(write('[bracket'), contains('k: "[bracket"'));
    });

    test('ordinary values are left bare', () {
      expect(write('done'), contains('k: done'));
      expect(write('2026-08-07'), contains('k: 2026-08-07'));
      expect(write('42'), contains('k: 42'));
      expect(write('a sentence with spaces'), contains('k: a sentence'));
      expect(write('https://example.com/x#y'), contains('k: https://'));
    });

    test('an embedded quote is escaped', () {
      final out = write('say "hi": now');
      expect(out, contains(r'k: "say \"hi\": now"'));
      expect(findSpan(out, 'k')!.displayValue, 'say "hi": now');
    });

    test('an empty value writes a bare key', () {
      expect(write(''), contains('k:\n'));
    });

    test('round-trips through the reader', () {
      for (final value in [
        'plain',
        'My Note: a study',
        '#hash',
        'true',
        ' padded ',
        'say "hi"',
      ]) {
        final out = write(value);
        expect(
          findSpan(out, 'k')!.displayValue,
          value,
          reason: 'writing then reading must return the same string',
        );
      }
    });
  });

  group('lists keep the form they were written in', () {
    const block =
        '---\n'
        'title: Notes\n'
        'tags:\n'
        '  - someday\n'
        '  - maybe\n'
        'publish: false\n'
        '---\n\nbody\n';

    const inline = '---\ntags: [homelab, project]\npublish: false\n---\nbody\n';

    test('a block list stays a block list', () {
      final out = setList(block, 'tags', ['someday', 'maybe', 'new']);
      expect(
        out,
        '---\n'
        'title: Notes\n'
        'tags:\n'
        '  - someday\n'
        '  - maybe\n'
        '  - new\n'
        'publish: false\n'
        '---\n\nbody\n',
      );
    });

    test('an inline list stays inline', () {
      final out = setList(inline, 'tags', ['homelab', 'project', 'new']);
      expect(out, contains('tags: [homelab, project, new]'));
      expect(out, contains('publish: false'));
    });

    test('removing an item from a block list keeps the neighbours', () {
      final out = setList(block, 'tags', ['maybe']);
      expect(out, contains('title: Notes'));
      expect(out, contains('publish: false'));
      expect(out, contains('  - maybe'));
      expect(out, isNot(contains('someday')));
    });

    test('the block list indentation is preserved', () {
      const fourSpace = '---\ntags:\n    - a\n---\nbody\n';
      final out = setList(fourSpace, 'tags', ['a', 'b']);
      expect(out, '---\ntags:\n    - a\n    - b\n---\nbody\n');
    });

    test('a brand-new list is written inline', () {
      final out = setList('---\nid: x\n---\nbody\n', 'tags', ['a', 'b']);
      expect(out, contains('tags: [a, b]'));
    });

    test('items needing quotes get them', () {
      final out = setList(inline, 'tags', ['a, b', 'plain']);
      expect(out, contains('tags: ["a, b", plain]'));
      expect(findSpan(out, 'tags')!.items, ['a, b', 'plain']);
    });

    test('an emptied block list leaves the key', () {
      final out = setList(block, 'tags', []);
      expect(out, contains('tags:'));
      expect(out, isNot(contains('- someday')));
      expect(out, contains('publish: false'));
    });
  });

  group('structures the editor refuses to touch', () {
    const nested = '---\nmeta:\n  a: 1\n  b: 2\nafter: x\n---\nbody\n';
    const blockScalar =
        '---\ndescription: |\n  id: not-a-key\n  more text\nreal: yes\n---\nb\n';

    test('a nested map is read-only', () {
      final span = findSpan(nested, 'meta')!;
      expect(span.form, PropertyForm.nested);
      expect(span.isEditable, isFalse);
      expect(
        setScalar(nested, 'meta', 'clobbered'),
        nested,
        reason: 'writing a scalar over a map would orphan its children',
      );
    });

    test('a block scalar is read-only', () {
      final span = findSpan(blockScalar, 'description')!;
      expect(span.form, PropertyForm.blockScalar);
      expect(span.isEditable, isFalse);
      expect(setScalar(blockScalar, 'description', 'x'), blockScalar);
    });

    test('a key inside a block scalar is not a property', () {
      // `id:` there is prose. Mistaking it would corrupt the note — the same
      // guard `line_of_key` has on the server.
      expect(findSpan(blockScalar, 'id'), isNull);
      expect(findSpan(blockScalar, 'real')!.displayValue, 'yes');
    });

    test('properties after a nested map are still found', () {
      expect(findSpan(nested, 'after')!.displayValue, 'x');
    });
  });

  group('rename and remove', () {
    const src = '---\na: 1\ntags:\n  - x\n  - y\nz: 26\n---\nbody\n';

    test('rename keeps the value, the form and the position', () {
      final out = renameKey(src, 'tags', 'labels');
      expect(out, '---\na: 1\nlabels:\n  - x\n  - y\nz: 26\n---\nbody\n');
    });

    test('rename keeps quoting untouched', () {
      const quoted = '---\nk: "a: b"\n---\nbody\n';
      expect(renameKey(quoted, 'k', 'j'), '---\nj: "a: b"\n---\nbody\n');
    });

    test('removing a block list takes its items and nothing else', () {
      final out = removeProperty(src, 'tags');
      expect(out, '---\na: 1\nz: 26\n---\nbody\n');
    });

    test('removing a scalar leaves its neighbours identical', () {
      final out = removeProperty(src, 'a');
      expect(out, '---\ntags:\n  - x\n  - y\nz: 26\n---\nbody\n');
    });

    test('removing a key that is not there changes nothing', () {
      expect(removeProperty(src, 'nope'), src);
    });
  });

  group('line endings and fences', () {
    test('a CRLF file stays CRLF', () {
      // The server's writer normalises these to LF. This one runs on ordinary
      // edits, so normalising would turn one property change into a
      // whole-file diff.
      const src = '---\r\nid: abc\r\nk: old\r\n---\r\nbody\r\n';
      final out = setScalar(src, 'k', 'new');
      expect(out, '---\r\nid: abc\r\nk: new\r\n---\r\nbody\r\n');
    });

    test('the body is returned byte for byte', () {
      const src = '---\nk: a\n---\n\n# H\n\n---\n\nnot frontmatter\n';
      final out = setScalar(src, 'k', 'b');
      expect(fm.split(out).body, fm.split(src).body);
    });

    test('a blank line inside the block is kept', () {
      const src = '---\na: 1\n\nb: 2\n---\nbody\n';
      final out = setScalar(src, 'b', '3');
      expect(out, '---\na: 1\n\nb: 3\n---\nbody\n');
    });
  });

  group('reading spans', () {
    test('every form is identified', () {
      const src =
          '---\n'
          'scalar: v\n'
          'inline: [a, b]\n'
          'block:\n'
          '  - a\n'
          'map:\n'
          '  k: v\n'
          'text: |\n'
          '  line\n'
          '---\nbody\n';

      final forms = {for (final s in readSpans(src)) s.key: s.form};
      expect(forms, {
        'scalar': PropertyForm.scalar,
        'inline': PropertyForm.inlineList,
        'block': PropertyForm.blockList,
        'map': PropertyForm.nested,
        'text': PropertyForm.blockScalar,
      });
    });

    test('an inline list with a quoted comma is split correctly', () {
      // The display-only reader flattens this wrong; the writer must not.
      const src = '---\ntags: [a, "b, c"]\n---\nbody\n';
      expect(findSpan(src, 'tags')!.items, ['a', 'b, c']);
    });

    test('a bare key with nothing under it is an empty scalar', () {
      const src = '---\nk:\nafter: x\n---\nbody\n';
      final span = findSpan(src, 'k')!;
      expect(span.form, PropertyForm.scalar);
      expect(span.displayValue, '');
      expect(span.isEditable, isTrue);
    });

    test('comment lines and blanks are not properties', () {
      const src = '---\n# just a comment\n\nreal: 1\n---\nbody\n';
      expect(readSpans(src).map((s) => s.key), ['real']);
    });

    test('a note with no frontmatter has no properties', () {
      expect(readSpans('# H\n\nbody\n'), isEmpty);
      expect(readSpans(''), isEmpty);
    });

    test('an unterminated block yields nothing', () {
      // It is body text, not metadata — same rule as the split.
      expect(readSpans('---\nid: abc\n\n# H\n'), isEmpty);
    });
  });

  group('the whole corpus survives an edit', () {
    // The same hostile inputs `frontmatter_test.dart` uses for split/join.
    final corpus = <String, String>{
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
      'crlf': '---\r\nid: abc\r\n---\r\nbody\r\n',
      'unicode': '---\ntitle: 日本語\n---\n\n# 見出し\n\n本文 🎉\n',
      'blank line in block': '---\na: 1\n\nb: 2\n---\nbody\n',
    };

    corpus.forEach((name, source) {
      test('adding a property to "$name" keeps the body and the old keys', () {
        final out = setScalar(source, 'storm_test_key', 'v');

        expect(
          fm.split(out).body,
          fm.split(source).body,
          reason: 'the body must be untouched',
        );
        for (final span in readSpans(source)) {
          expect(
            findSpan(out, span.key)?.displayValue,
            span.displayValue,
            reason: 'property "${span.key}" changed',
          );
        }
        expect(findSpan(out, 'storm_test_key')!.displayValue, 'v');
      });

      test('removing the added property restores "$name" exactly', () {
        final added = setScalar(source, 'storm_test_key', 'v');
        expect(removeProperty(added, 'storm_test_key'), source);
      });
    });
  });
}
