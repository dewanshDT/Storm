import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:storm/state/health.dart';

String _version(String path, RegExp line) =>
    line.firstMatch(File(path).readAsStringSync())!.group(1)!;

void main() {
  test('a source build of client and server from one commit is compatible', () {
    final client = _version(
      'pubspec.yaml',
      RegExp(r'^version:\s*(\S+)', multiLine: true),
    );
    final server = _version(
      '../server/Cargo.toml',
      RegExp(r'^version\s*=\s*"([^"]+)"', multiLine: true),
    );
    expect(
      versionsCompatible(client, server),
      isTrue,
      reason: '$client vs $server',
    );
  });

  test('release stamps compare by major.minor', () {
    expect(versionsCompatible('0.3.1+40', '0.3.0'), isTrue);
    expect(versionsCompatible('v0.3.1', '0.3.1'), isTrue);
    expect(versionsCompatible('0.3.1+40', '0.4.0'), isFalse);
    expect(versionsCompatible('1.0.0+1', '0.1.0'), isFalse);
    expect(versionsCompatible('0.1.0+1', '1.1.0'), isFalse);
    expect(versionsCompatible('dev', '0.1.0'), isFalse);
  });
}
