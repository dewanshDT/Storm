import 'dart:io';

/// What "shared" means: the client's generic building blocks, not its screens
/// or the widgets that belong to one feature. Paths are relative to the
/// client's `lib/`. The coverage test reads this same list.
const sharedFiles = [
  'ui/controls.dart',
  'ui/widgets.dart',
  'ui/surfaces.dart',
  'ui/panels.dart',
  'ui/states.dart',
  'ui/session_status.dart',
  'ui/accents.dart',
  'ui/icons.dart',
  'ui/settings/settings_widgets.dart',
  'ui/shell/sidebar_rows.dart',
];

/// The gallery instantiates every component by design, so it is not usage.
const excludedFiles = ['ui/gallery_screen.dart'];

enum Kind { inkWell, gestureDetector, boxDecoration }

class Site {
  const Site(this.file, this.line, [this.kind]);

  final String file;
  final int line;
  final Kind? kind;
}

class Usage {
  const Usage(this.components, this.candidates);

  final Map<String, List<Site>> components;
  final List<Site> candidates;
}

Future<void> main(List<String> args) async {
  var client = '../client';
  var out = 'lib/usage.g.dart';
  for (var i = 0; i < args.length; i++) {
    if (args[i] == '--client') client = args[++i];
    if (args[i] == '--out') out = args[++i];
  }
  final usage = scan(Directory('$client/lib'));
  File(out).writeAsStringSync(render(usage));
  final used = usage.components.values.where((s) => s.isNotEmpty).length;
  stdout.writeln(
    '${usage.components.length} components ($used used), '
    '${usage.candidates.length} one-off candidates -> $out',
  );
}

Usage scan(Directory lib) {
  final files = <String, String>{
    for (final f in lib.listSync(recursive: true).whereType<File>())
      if (f.path.endsWith('.dart') && !_generated(f.path))
        _relative(lib, f): scrub(f.readAsStringSync()),
  };
  final defined = <String, String>{};
  for (final shared in sharedFiles) {
    final text = files[shared];
    if (text == null) continue;
    for (final m in _widgetClass.allMatches(text)) {
      defined[m.group(1)!] = shared;
    }
  }

  final components = <String, List<Site>>{};
  final names = defined.keys.toList()..sort();
  for (final name in names) {
    final call = RegExp(
      '(?<![\\w\$.])$name\\s*(?:<[^<>()]*(?:<[^<>()]*>[^<>()]*)*>)?'
      '\\s*(?:\\.\\w+)?\\s*\\(',
    );
    final sites = <Site>[];
    for (final e in files.entries) {
      if (e.key == defined[name] || excludedFiles.contains(e.key)) continue;
      final lines = _Lines(e.value);
      for (final m in call.allMatches(e.value)) {
        sites.add(Site(e.key, lines.of(m.start)));
      }
    }
    components[name] = _sorted(sites);
  }

  final candidates = <Site>[];
  for (final e in files.entries) {
    if (sharedFiles.contains(e.key) || excludedFiles.contains(e.key)) continue;
    candidates.addAll(_candidates(e.key, e.value));
  }
  return Usage(components, _sorted(candidates));
}

final _widgetClass = RegExp(
  r'^[ \t]*(?:(?:abstract|final|base)\s+)*class\s+([A-Z]\w*)\s*(?:<[^{]*?>)?\s*'
  r'extends\s+(?:Stateless|Stateful)Widget\b',
  multiLine: true,
);

final _candidateStart = RegExp(
  r'(?<![\w$.])(InkWell|GestureDetector|Container|DecoratedBox)\s*\(',
);

List<Site> _candidates(String file, String text) {
  final lines = _Lines(text);
  final sites = <Site>[];
  for (final m in _candidateStart.allMatches(text)) {
    final name = m.group(1)!;
    final line = lines.of(m.start);
    if (name == 'InkWell') {
      sites.add(Site(file, line, Kind.inkWell));
    } else if (name == 'GestureDetector') {
      sites.add(Site(file, line, Kind.gestureDetector));
    } else if (_ownArguments(text, m.end - 1).contains(_boxDecoration)) {
      sites.add(Site(file, line, Kind.boxDecoration));
    }
  }
  return sites;
}

final _boxDecoration = RegExp(r'\bBoxDecoration\b');

/// The call's arguments with everything inside nested brackets dropped, so a
/// child's decoration is not attributed to its parent.
String _ownArguments(String text, int open) {
  final b = StringBuffer();
  var depth = 0;
  for (var i = open; i < text.length; i++) {
    final c = text[i];
    if (c == '(' || c == '[' || c == '{') {
      depth++;
      if (depth > 1) continue;
    } else if (c == ')' || c == ']' || c == '}') {
      depth--;
      if (depth == 0) break;
      continue;
    }
    if (depth == 1) b.write(c);
  }
  return b.toString();
}

List<Site> _sorted(List<Site> sites) => sites
  ..sort((a, b) {
    final f = a.file.compareTo(b.file);
    return f != 0 ? f : a.line.compareTo(b.line);
  });

bool _generated(String path) =>
    path.endsWith('.g.dart') || path.endsWith('.freezed.dart');

String _relative(Directory lib, File f) {
  final root = lib.path.endsWith('/') ? lib.path : '${lib.path}/';
  return f.path.substring(root.length).replaceAll('\\', '/');
}

class _Lines {
  _Lines(String text) {
    for (var i = 0; i < text.length; i++) {
      if (text.codeUnitAt(i) == 10) _breaks.add(i);
    }
  }

  final _breaks = <int>[];

  int of(int offset) {
    var lo = 0, hi = _breaks.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (_breaks[mid] < offset) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return lo + 1;
  }
}

String render(Usage usage) {
  final b = StringBuffer()
    ..writeln('// Generated by tool/ui_usage.dart; do not edit.')
    ..writeln("import 'usage.dart';")
    ..writeln()
    ..writeln('const componentUsage = <String, List<UsageSite>>{');
  for (final e in usage.components.entries) {
    if (e.value.isEmpty) {
      b.writeln("  '${e.key}': <UsageSite>[],");
      continue;
    }
    b.writeln("  '${e.key}': [");
    for (final s in e.value) {
      b.writeln("    UsageSite('${s.file}', ${s.line}),");
    }
    b.writeln('  ],');
  }
  b
    ..writeln('};')
    ..writeln()
    ..writeln('const oneOffCandidates = <CandidateSite>[');
  for (final s in usage.candidates) {
    b.writeln(
      "  CandidateSite('${s.file}', ${s.line}, CandidateKind.${s.kind!.name}),",
    );
  }
  b.writeln('];');
  return b.toString();
}

/// Blanks comments and string contents, keeping offsets and newlines, so a
/// match in the result is a match in code. `${...}` inside a string is code
/// and is kept.
String scrub(String src) {
  final b = src.codeUnits.toList();
  _code(src, b, 0, false);
  return String.fromCharCodes(b);
}

void _blank(List<int> b, int from, int to) {
  for (var i = from; i < to && i < b.length; i++) {
    if (b[i] != 10) b[i] = 32;
  }
}

bool _ident(int c) =>
    (c >= 48 && c <= 57) ||
    (c >= 65 && c <= 90) ||
    (c >= 97 && c <= 122) ||
    c == 95 ||
    c == 36;

int _code(String s, List<int> b, int start, bool inInterpolation) {
  var i = start;
  var depth = 0;
  while (i < s.length) {
    final c = s[i];
    final n = i + 1 < s.length ? s[i + 1] : '';
    if (c == '/' && n == '/') {
      var j = i;
      while (j < s.length && s[j] != '\n') {
        j++;
      }
      _blank(b, i, j);
      i = j;
    } else if (c == '/' && n == '*') {
      var j = i + 2;
      var nest = 1;
      while (j < s.length && nest > 0) {
        if (s.startsWith('/*', j)) {
          nest++;
          j += 2;
        } else if (s.startsWith('*/', j)) {
          nest--;
          j += 2;
        } else {
          j++;
        }
      }
      _blank(b, i, j);
      i = j;
    } else if (c == "'" || c == '"') {
      final raw = i > 0 && s[i - 1] == 'r' && (i < 2 || !_ident(b[i - 2]));
      i = _string(s, b, i, raw);
    } else if (inInterpolation && c == '{') {
      depth++;
      i++;
    } else if (inInterpolation && c == '}') {
      if (depth == 0) return i;
      depth--;
      i++;
    } else {
      i++;
    }
  }
  return i;
}

int _string(String s, List<int> b, int start, bool raw) {
  final q = s[start];
  final triple = s.startsWith(q * 3, start);
  final open = triple ? 3 : 1;
  var i = start + open;
  var from = start;
  while (i < s.length) {
    if (!raw && s[i] == '\\') {
      i += 2;
    } else if (triple ? s.startsWith(q * 3, i) : s[i] == q) {
      i += open;
      _blank(b, from, i);
      return i;
    } else if (!triple && s[i] == '\n') {
      break;
    } else if (!raw && s[i] == r'$' && i + 1 < s.length && s[i + 1] == '{') {
      _blank(b, from, i + 2);
      i = _code(s, b, i + 2, true);
      from = i;
      i++;
    } else {
      i++;
    }
  }
  _blank(b, from, i);
  return i;
}
