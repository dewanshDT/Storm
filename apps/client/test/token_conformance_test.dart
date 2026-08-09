import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// A source scan over `lib/ui/`, guarding the one rule the token layer rests
/// on: no screen decides a size, a radius or a colour for itself.
///
/// A source scan rather than a widget test because the failure it catches is
/// invisible at runtime in the default theme — a literal `circular(8)` looks
/// right under Storm dark, where `rControl` is 10, and only goes wrong under
/// SlowFlow, where `rCard` is 2 and every hardcoded corner stays round while
/// the cards go square.
void main() {
  final files = Directory('lib/ui')
      .listSync(recursive: true)
      .whereType<File>()
      .where((f) => f.path.endsWith('.dart'))
      .toList();

  /// Lines that are not comments, with their 1-based number.
  Iterable<(int, String)> codeLines(File f) sync* {
    var n = 0;
    for (final line in f.readAsLinesSync()) {
      n++;
      final trimmed = line.trimLeft();
      if (trimmed.startsWith('//')) continue;
      yield (n, line);
    }
  }

  test('lib/ui has at least the files this suite thinks it has', () {
    expect(files.length, greaterThan(20));
  });

  test('no literal Colors.*', () {
    // `Colors.transparent` is a name for `0x00000000`, not a palette choice.
    final offenders = <String>[];
    for (final f in files) {
      for (final (n, line) in codeLines(f)) {
        if (RegExp(r'\bColors\.(?!transparent)').hasMatch(line)) {
          offenders.add('${f.path}:$n');
        }
      }
    }
    expect(offenders, isEmpty, reason: 'use a StormTokens colour');
  });

  test('no OutlineInputBorder overriding the themed input', () {
    // Passing one discards the fill, the stroke and the rControl radius that
    // `inputDecorationTheme` supplies, leaving a 4px Material default box in
    // the middle of a themed screen. Three call sites did this.
    final offenders = <String>[];
    for (final f in files) {
      // theme.dart is where the themed border is *defined*.
      if (f.path.endsWith('theme.dart')) continue;
      for (final (n, line) in codeLines(f)) {
        if (line.contains('OutlineInputBorder')) offenders.add('${f.path}:$n');
      }
    }
    expect(offenders, isEmpty, reason: 'use StormInput, or let the theme win');
  });

  test('no hardcoded font sizes', () {
    // theme.dart builds the TextTheme from the tokens and markdown_theme takes
    // a base style; everywhere else a number here is a size that cannot move.
    final offenders = <String>[];
    for (final f in files) {
      for (final (n, line) in codeLines(f)) {
        if (RegExp(r'fontSize:\s*[0-9]').hasMatch(line)) {
          offenders.add('${f.path}:$n');
        }
      }
    }
    expect(offenders, isEmpty, reason: 'derive from StormTokens');
  });

  test('no hardcoded radii except the pill', () {
    // 999 is the pill, which is a shape rather than a size — the nav bubble
    // and the tag chip are round at every scale.
    final offenders = <String>[];
    for (final f in files) {
      for (final (n, line) in codeLines(f)) {
        for (final m in RegExp(
          r'circular\(\s*([0-9.]+)\s*\)',
        ).allMatches(line)) {
          if (m.group(1) != '999') offenders.add('${f.path}:$n');
        }
      }
    }
    expect(offenders, isEmpty, reason: 'use rCard or rControl');
  });
}
