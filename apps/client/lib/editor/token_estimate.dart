import 'frontmatter.dart' as fm;

/// A rough size of some text in model tokens, words and characters.
///
/// Decision 158: no tokenizer ships with the client, so tokens are a
/// character-class heuristic calibrated against cl100k_base (see
/// `test/token_estimate_test.dart`). Good to about ±15%; the exact count
/// depends on the model.
class TokenEstimate {
  const TokenEstimate({
    required this.tokens,
    required this.words,
    required this.chars,
  });

  final int tokens;
  final int words;
  final int chars;

  static const zero = TokenEstimate(tokens: 0, words: 0, chars: 0);

  @override
  bool operator ==(Object other) =>
      other is TokenEstimate &&
      other.tokens == tokens &&
      other.words == words &&
      other.chars == chars;

  @override
  int get hashCode => Object.hash(tokens, words, chars);

  @override
  String toString() => 'TokenEstimate($tokens, $words, $chars)';
}

const double kLettersPerToken = 7.0;
const double kSymbolsPerToken = 1.5;
const double kCodePerToken = 3.4;
const double kWhitespaceRunsPerToken = 2.0;

const kSizeTooltip =
    'Estimate; the exact count depends on the model, about ±15%.';

/// Notes above this many characters are estimated off the UI thread.
const int kLargeNoteChars = 200 * 1024;

/// Estimates [text]: tokens, words and characters all over the same string.
TokenEstimate estimateText(String text) {
  var cjk = 0, letters = 0, symbols = 0, code = 0, wsRuns = 0, words = 0;
  var inFence = false;
  for (final line in text.split('\n')) {
    final fence = line.trimLeft().startsWith('```');
    final isCode = inFence || fence;
    if (fence) inFence = !inFence;
    var prevSpace = false;
    var inWord = false;
    for (final r in line.runes) {
      if (_isSpace(r)) {
        if (!prevSpace) wsRuns++;
        prevSpace = true;
        inWord = false;
        continue;
      }
      prevSpace = false;
      if (_isCjk(r)) {
        cjk++;
        words++;
        inWord = false;
      } else {
        if (!inWord) words++;
        inWord = true;
        if (isCode) {
          code++;
        } else if (_isLetter(r)) {
          letters++;
        } else {
          symbols++;
        }
      }
    }
    // The line's newline.
    if (!prevSpace) wsRuns++;
  }
  final tokens =
      cjk +
      letters / kLettersPerToken +
      symbols / kSymbolsPerToken +
      code / kCodePerToken +
      wsRuns / kWhitespaceRunsPerToken;
  return TokenEstimate(
    tokens: text.isEmpty ? 0 : tokens.round(),
    words: words,
    chars: text.runes.length,
  );
}

/// A note's size: tokens over the whole file, as an agent's `get_note`
/// returns it, and words and characters over the body a person reads.
TokenEstimate estimateNote(String wholeFile) {
  final body = fm.split(wholeFile).body;
  final file = estimateText(wholeFile);
  if (body.length == wholeFile.length) return file;
  final b = estimateText(body);
  return TokenEstimate(tokens: file.tokens, words: b.words, chars: b.chars);
}

/// `~240`, `~2.4k`, `~1.2M`.
String formatTokens(int n) => '~${compactCount(n)}';

/// `240`, `2.4k`, `10.2k`, `240k`, `1.2M`.
String compactCount(int n) {
  if (n < 1000) return '$n';
  if (n < 999500) return '${_oneDecimal(n / 1000)}k';
  return '${_oneDecimal(n / 1000000)}M';
}

/// `1,812`.
String groupedCount(int n) {
  final s = '$n';
  final out = StringBuffer();
  for (var i = 0; i < s.length; i++) {
    if (i > 0 && (s.length - i) % 3 == 0) out.write(',');
    out.write(s[i]);
  }
  return out.toString();
}

String _oneDecimal(double v) {
  if (v >= 100) return v.round().toString();
  final s = v.toStringAsFixed(1);
  return s.endsWith('.0') ? s.substring(0, s.length - 2) : s;
}

bool _isSpace(int r) =>
    r == 0x20 ||
    (r >= 0x09 && r <= 0x0D) ||
    r == 0x85 ||
    r == 0xA0 ||
    r == 0x1680 ||
    (r >= 0x2000 && r <= 0x200A) ||
    r == 0x2028 ||
    r == 0x2029 ||
    r == 0x202F ||
    r == 0x205F ||
    r == 0x3000;

bool _isCjk(int r) =>
    (r >= 0x1100 && r <= 0x11FF) ||
    (r >= 0x3000 && r <= 0x30FF) ||
    (r >= 0x3130 && r <= 0x318F) ||
    (r >= 0x3400 && r <= 0x4DBF) ||
    (r >= 0x4E00 && r <= 0x9FFF) ||
    (r >= 0xAC00 && r <= 0xD7AF) ||
    (r >= 0xF900 && r <= 0xFAFF) ||
    (r >= 0xFF00 && r <= 0xFFEF) ||
    (r >= 0x20000 && r <= 0x2FA1F);

bool _isLetter(int r) =>
    (r >= 0x41 && r <= 0x5A) ||
    (r >= 0x61 && r <= 0x7A) ||
    (r >= 0xC0 && r <= 0x24F && r != 0xD7 && r != 0xF7) ||
    (r >= 0x370 && r <= 0x52F);
