/// How well a query matches a piece of text (decision 156i), best first.
enum MatchTier { exact, prefix, wordStart, substring, subsequence }

/// One string an item can be matched on. Fields are tried in order, and an
/// earlier field's match beats any later one's; [worst] is the loosest tier
/// the field accepts, so a long path or a host name never matches on a
/// scattered subsequence.
class MatchField {
  const MatchField(this.text, {this.worst = MatchTier.subsequence});

  final String text;
  final MatchTier worst;
}

/// How [query] matches [text], case-insensitively, or null for not at all.
/// [query] is expected trimmed and lower-cased.
MatchTier? matchTier(String text, String query) {
  if (query.isEmpty) return null;
  final hay = text.toLowerCase();
  if (hay == query) return MatchTier.exact;
  if (hay.startsWith(query)) return MatchTier.prefix;
  if (_atWordStart(hay, query)) return MatchTier.wordStart;
  if (hay.contains(query)) return MatchTier.substring;
  if (_subsequence(hay, query)) return MatchTier.subsequence;
  return null;
}

bool _isWordBreak(String c) => ' -_/.:·'.contains(c);

/// [query] starts at a word, or each of its words starts one of [hay]'s in
/// order ("des sys" for "Design System").
bool _atWordStart(String hay, String query) {
  if (hay.length > 1) {
    var i = hay.indexOf(query, 1);
    while (i > 0) {
      if (_isWordBreak(hay[i - 1])) return true;
      i = hay.indexOf(query, i + 1);
    }
  }
  final parts = query.split(RegExp(r'\s+')).where((p) => p.isNotEmpty);
  if (parts.length < 2) return false;
  final words = hay.split(RegExp(r'[\s\-_/.:·]+'));
  var w = 0;
  for (final p in parts) {
    while (w < words.length && !words[w].startsWith(p)) {
      w++;
    }
    if (w == words.length) return false;
    w++;
  }
  return true;
}

bool _subsequence(String hay, String query) {
  var i = 0;
  for (var j = 0; j < hay.length && i < query.length; j++) {
    if (hay[j] == query[i]) i++;
  }
  return i == query.length;
}

/// An item's best match: which field, how well, and how long that field is.
class Scored<T> {
  const Scored(this.item, this.field, this.tier, this.text);

  final T item;
  final int field;
  final MatchTier tier;
  final String text;
}

/// [item]'s best match on [fields], or null when none matches.
Scored<T>? scoreOf<T>(T item, List<MatchField> fields, String query) {
  for (final (i, f) in fields.indexed) {
    final tier = matchTier(f.text, query);
    if (tier != null && tier.index <= f.worst.index) {
      return Scored(item, i, tier, f.text);
    }
  }
  return null;
}

/// The order of two matches: field, then tier; then [tieBreak] (frecency,
/// the active vault), which never lifts an item over a better tier; then the
/// shorter text, then alphabetically, so the order never jitters between
/// keystrokes.
int compareScored<T>(
  Scored<T> a,
  Scored<T> b, {
  int Function(T a, T b)? tieBreak,
}) {
  if (a.field != b.field) return a.field - b.field;
  if (a.tier != b.tier) return a.tier.index - b.tier.index;
  final tie = tieBreak?.call(a.item, b.item) ?? 0;
  if (tie != 0) return tie;
  if (a.text.length != b.text.length) return a.text.length - b.text.length;
  return a.text.toLowerCase().compareTo(b.text.toLowerCase());
}

/// [items] that match [query], best first. [query] is trimmed and
/// lower-cased here.
List<T> rankMatches<T>(
  Iterable<T> items,
  String query,
  List<MatchField> Function(T item) fields, {
  int Function(T a, T b)? tieBreak,
  int? limit,
}) {
  final want = query.trim().toLowerCase();
  if (want.isEmpty) return const [];
  final scored = <Scored<T>>[
    for (final item in items) ?scoreOf(item, fields(item), want),
  ]..sort((a, b) => compareScored(a, b, tieBreak: tieBreak));
  final out = [for (final s in scored) s.item];
  return limit == null || out.length <= limit ? out : out.sublist(0, limit);
}
