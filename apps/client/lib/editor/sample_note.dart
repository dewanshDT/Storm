import 'dart:math';

/// Generates a realistic markdown document of roughly [lines] lines.
///
/// Two properties matter for the perf gate, and getting either wrong makes the
/// benchmark lie:
///
///  * **Span density.** Cost tracks the number of styled runs, not lines. Plain
///    prose would understate it badly, so this mixes headings, emphasis,
///    wikilinks, tags, code and fences at roughly real-vault density.
///  * **Line uniqueness.** The controller caches spans keyed by line content.
///    A document that repeats the same block collapses to a handful of unique
///    lines and makes the cache look far better than it is. Every line here
///    gets unique text.
String sampleNote(int lines, {int seed = 42}) {
  final rng = Random(seed);
  final buf = StringBuffer()
    ..writeln('---')
    ..writeln('id: 8f3a2c10-4b1e-4a7c-9d2f-1a2b3c4d5e6f')
    ..writeln('created: 2026-08-05T10:00:00Z')
    ..writeln('modified: 2026-08-05T10:04:12Z')
    ..writeln('tags: [homelab, storm, spike]')
    ..writeln('---')
    ..writeln();

  const nouns = [
    'vault',
    'note',
    'server',
    'client',
    'cache',
    'outbox',
    'merge',
    'index',
    'watcher',
    'digest',
    'frontmatter',
    'attachment',
    'wikilink',
    'tag',
    'snapshot',
    'replica',
    'checkpoint',
    'transaction',
    'manifest',
    'cursor',
  ];
  const verbs = [
    'reconciles',
    'replays',
    'indexes',
    'rewrites',
    'broadcasts',
    'debounces',
    'hashes',
    'prunes',
    'resolves',
    'streams',
    'validates',
    'coalesces',
  ];
  String noun() => nouns[rng.nextInt(nouns.length)];
  String verb() => verbs[rng.nextInt(verbs.length)];

  var written = 7;
  var n = 0;

  while (written < lines) {
    n++;
    buf.writeln('## Section $n — how the ${noun()} ${verb()}');
    buf.writeln();
    buf.writeln(
      'The **${noun()} $n** ${verb()} against the *${noun()}* before it '
      '${verb()}. See [[${noun().toUpperCase()} $n]] and '
      '[[Design Note ${n * 3}]]. #homelab/${noun()}',
    );
    buf.writeln();
    buf.writeln('- Entry ${n * 7}: `${noun()}_$n()` ${verb()} the ${noun()}');
    buf.writeln('- Entry ${n * 7 + 1}: ~~${noun()} $n~~ became ==${noun()}==');
    buf.writeln('- Entry ${n * 7 + 2}: see [ref $n](https://d.dev/$n) #t$n');
    buf.writeln('1. Step ${n * 2}: client sends `base_version=$n`');
    buf.writeln('2. Step ${n * 2 + 1}: server ${verb()} into the ${noun()}');
    buf.writeln();
    buf.writeln('> Note $n: the ${noun()} ${verb()} only on reconnect, so the');
    buf.writeln('> ${noun()} $n never ${verb()} twice. #design/note$n');
    buf.writeln();
    buf.writeln('### Detail $n');
    buf.writeln();
    buf.writeln('```rust');
    buf.writeln('fn ${noun()}_$n(base: &str) -> Result<String, Error> {');
    buf.writeln('    let ${noun()} = ${noun()}::load($n)?;');
    buf.writeln('    Ok(${noun()}.${verb().replaceAll('s', '')}(base))');
    buf.writeln('}');
    buf.writeln('```');
    buf.writeln();
    buf.writeln(
      'Plain prose paragraph number $n carrying no markup whatsoever, which '
      'keeps the span density honest for section ${n * 11}.',
    );
    buf.writeln();
    written += 24;
  }

  // Trim to exactly [lines]. Without this the generator overshoots by up to a
  // section, which is enough to cross the controller's degrade threshold and
  // silently turn a styled benchmark into an unstyled one.
  final out = buf.toString().split('\n');
  return out.sublist(0, min(lines, out.length)).join('\n');
}
