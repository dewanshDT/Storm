import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:storm/agent/ntfy_topic.dart';

/// Decision 161, slice 161-9: "Generate private topic"'s own randomness —
/// length and alphabet, and that two calls don't collide.
void main() {
  test('the random part is exactly kNtfyTopicRandomLength characters', () {
    final topic = randomNtfyTopic(random: Random(1));
    expect(topic, startsWith('storm-'));
    expect(topic.substring('storm-'.length).length, kNtfyTopicRandomLength);
  });

  test('every character comes from the base32 alphabet', () {
    final topic = randomNtfyTopic(random: Random(42));
    final random = topic.substring('storm-'.length);
    for (final c in random.split('')) {
      expect(
        kNtfyTopicAlphabet.contains(c),
        isTrue,
        reason: '"$c" is not in $kNtfyTopicAlphabet',
      );
    }
    // Exactly the base32 rule: lowercase letters and digits only, no
    // ambiguous characters like 0/1/8/9 or uppercase.
    expect(RegExp(r'^[a-z2-7]+$').hasMatch(random), isTrue);
  });

  test('the alphabet itself is 32 characters with no duplicates', () {
    expect(kNtfyTopicAlphabet.length, 32);
    expect(kNtfyTopicAlphabet.split('').toSet().length, 32);
  });

  test('randomNtfyUrl wraps the topic in the ntfy.sh host', () {
    final url = randomNtfyUrl(random: Random(7));
    expect(url, startsWith('https://ntfy.sh/storm-'));
    final uri = Uri.parse(url);
    expect(uri.scheme, 'https');
    expect(uri.host, 'ntfy.sh');
    expect(uri.pathSegments, hasLength(1));
  });

  test('two generated topics never collide in a large sample', () {
    final seen = <String>{};
    for (var i = 0; i < 5000; i++) {
      seen.add(randomNtfyTopic());
    }
    expect(seen.length, 5000);
  });

  test('a fixed seed is reproducible (confirms the RNG is actually used)', () {
    expect(
      randomNtfyTopic(random: Random(99)),
      randomNtfyTopic(random: Random(99)),
    );
  });
}
