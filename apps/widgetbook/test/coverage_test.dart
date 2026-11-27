import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import '../tool/ui_usage.dart' show sharedFiles;

/// Shared widgets that cannot be rendered on their own, with why.
const _allowed = <String, String>{
  'SettingsPage':
      'a full page: mounts StormScaffold and the go_router back route, so it '
      'only renders inside the whole app shell',
};

final _widget = RegExp(
  r'^class\s+([A-Z]\w*)(?:<[^{]*?>)?\s+extends\s+'
  r'(?:StatelessWidget|StatefulWidget|ConsumerWidget|ConsumerStatefulWidget|HookWidget)\b',
  multiLine: true,
);

final _useCase = RegExp(
  r'''UseCase\(\s*name:\s*(?:'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*")\s*,\s*type:\s*([A-Z]\w*)''',
);

Set<String> _names(String source, RegExp pattern) =>
    pattern.allMatches(source).map((m) => m.group(1)!).toSet();

void main() {
  final shared = <String>{
    for (final file in sharedFiles)
      ..._names(File('../client/lib/$file').readAsStringSync(), _widget),
  };
  final covered = <String>{
    for (final file in Directory('lib/use_cases').listSync().whereType<File>())
      if (file.path.endsWith('.dart'))
        ..._names(file.readAsStringSync(), _useCase),
  };

  test('the scanned files are found and hold widgets', () {
    expect(shared, containsAll(['StormButton', 'StatusDot', 'EmptyState']));
    expect(covered, containsAll(['StormButton', 'EmptyState']));
  });

  test('every shared widget has a use case or is allow-listed', () {
    final missing = (shared.difference(
      covered,
    )..removeAll(_allowed.keys)).toList()..sort();
    expect(
      missing,
      isEmpty,
      reason:
          'No use case for: ${missing.join(', ')}. Add one under '
          'lib/use_cases, or allow-list it in test/coverage_test.dart '
          'with a reason.',
    );
  });

  test('the allow-list holds no stale entries', () {
    final gone = _allowed.keys.where((n) => !shared.contains(n)).toList();
    final done = _allowed.keys.where(covered.contains).toList();
    expect(gone, isEmpty, reason: 'No longer a shared widget: $gone');
    expect(done, isEmpty, reason: 'Now has a use case, drop it: $done');
  });
}
