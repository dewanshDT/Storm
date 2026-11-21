/// Decision 161, slice 161-9: "Generate private topic" fills the ntfy URL
/// field with a fresh, hard-to-guess topic on `ntfy.sh`. A topic is a
/// public, unauthenticated pub/sub channel — anyone who learns its name can
/// read or publish to it — so the only privacy a free ntfy.sh topic has is
/// a name nobody else can guess.
library;

import 'dart:math';

/// RFC 4648 base32, lowercase, no padding characters: Storm's own topics
/// never need the `=` padding or mixed case, and lowercase reads better in
/// a URL most people will never type by hand.
const kNtfyTopicAlphabet = 'abcdefghijklmnopqrstuvwxyz234567';

/// How many random characters follow the `storm-` prefix.
const kNtfyTopicRandomLength = 22;

/// `storm-<22 random base32 chars>` — the bare topic name, no host.
String randomNtfyTopic({Random? random}) {
  final rng = random ?? Random.secure();
  final chars = List<String>.generate(
    kNtfyTopicRandomLength,
    (_) => kNtfyTopicAlphabet[rng.nextInt(kNtfyTopicAlphabet.length)],
  );
  return 'storm-${chars.join()}';
}

/// The full `https://ntfy.sh/storm-…` URL "Generate private topic" fills in.
String randomNtfyUrl({Random? random}) =>
    'https://ntfy.sh/${randomNtfyTopic(random: random)}';
