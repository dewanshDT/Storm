import 'dart:math';

/// Replays the edit made to [sent] while it was being saved — the difference
/// between [sent] and [edited] — onto [server], the text the server answered
/// with (decision 128b).
///
/// The edit is found by the text around it in [sent]. Returns null when that
/// text is not in [server] exactly once: putting the user's words in the
/// wrong place would be worse than letting the server's three-way merge
/// place them, which is what the caller does instead.
String? replayEdit({
  required String sent,
  required String edited,
  required String server,
}) {
  if (sent == edited) return server;
  if (sent.isEmpty) return null;

  final shorter = min(sent.length, edited.length);
  var prefix = 0;
  while (prefix < shorter &&
      sent.codeUnitAt(prefix) == edited.codeUnitAt(prefix)) {
    prefix++;
  }
  var suffix = 0;
  while (suffix < sent.length - prefix &&
      suffix < edited.length - prefix &&
      sent.codeUnitAt(sent.length - 1 - suffix) ==
          edited.codeUnitAt(edited.length - 1 - suffix)) {
    suffix++;
  }
  final removed = sent.substring(prefix, sent.length - suffix);
  final inserted = edited.substring(prefix, edited.length - suffix);

  // Widest context first: it is the most likely to be unique. Narrower
  // context is tried only when the wide one is absent, since it may reach
  // into a region the other writer changed; a context that occurs twice
  // only gets more ambiguous as it shrinks.
  for (final reach in const [64, 32, 16, 8]) {
    final before = sent.substring(max(0, prefix - reach), prefix);
    final afterStart = sent.length - suffix;
    final after = sent.substring(
      afterStart,
      min(sent.length, afterStart + reach),
    );
    final anchor = '$before$removed$after';
    if (anchor.isEmpty) return null;
    final at = server.indexOf(anchor);
    if (at < 0) continue;
    if (server.indexOf(anchor, at + 1) >= 0) return null;
    final start = at + before.length;
    return server.replaceRange(start, start + removed.length, inserted);
  }
  return null;
}
