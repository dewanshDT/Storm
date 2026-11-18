import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:xterm2/xterm.dart' show TerminalView;

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/state/terminal_prefs.dart';
import 'package:storm/ui/oklch.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/themes/theme_audit.dart';
import 'package:storm/ui/themes/theme_config.dart';
import 'package:storm/ui/themes/theme_library.dart';
import 'package:storm/ui/tokens.dart';

/// Decision 160: theme families from config, the (family, mode) setting, and
/// the derived terminal palette.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  tearDown(() => ThemeLibrary.families.value = ThemeLibrary.builtIn);

  Map<String, Object?> config() => {
    'id': 'test',
    'label': 'Test',
    'inspiredBy': {
      'name': 'Example',
      'url': 'https://example.com',
      'licence': 'MIT',
    },
    'shape': 'storm',
    'variants': {
      'light': {
        'bgL': 94,
        'hue': 250,
        'chroma': 10,
        'accentH': 250,
        'accentC': 0.12,
        'accentL': 55,
      },
      'dark': {
        'label': 'Test night',
        'bgL': 22,
        'hue': 250,
        'chroma': 20,
        'accentH': 250,
        'accentC': 0.12,
        'accentL': 72,
        'semantic': {'amber': 80, 'green': 140, 'danger': 15},
        'ansiHues': List.filled(16, 200.0),
        'ansiChroma': 0.1,
      },
    },
  };

  ThemeFamily parse(Object? json) =>
      parseThemeFamily(jsonEncode(json), source: 'test');

  Future<List<ThemeFamily>> loadAssets() async {
    await ThemeLibrary.load(rootBundle);
    return ThemeLibrary.families.value;
  }

  List<Color> colours(StormTokens t) => [
    t.bg,
    t.surface,
    t.surface2,
    t.border,
    t.text,
    t.text2,
    t.text3,
    t.accent,
    t.accentSoft,
    t.onAccent,
    t.amber,
    t.amberSoft,
    t.green,
    t.danger,
  ];

  group('configs', () {
    test('a good config parses, with defaults filled in', () {
      final f = parse(config());
      expect(f.id, 'test');
      expect(f.inspiredBy?.name, 'Example');
      expect(f.light!.label, 'Test light');
      expect(f.dark!.label, 'Test night');
      expect(f.light!.inputs.amberHue, 68);
      expect(f.dark!.inputs.dangerHue, 15);
      expect(f.light!.inputs.ansi, AnsiInputs.storm);
      expect(f.dark!.inputs.ansi.chroma, everyElement(0.1));
      expect(f.dark!.inputs.shadow, 35);
      expect(f.light!.inputs.shadow, 20);
      expect(f.light!.inputs.scale, 1.25);
    });

    test('ANSI hues can be named slots', () {
      final json = config();
      final dark = (json['variants']! as Map)['dark'] as Map;
      dark['ansiHues'] = {
        for (final (i, n) in AnsiInputs.names.indexed)
          n.replaceAll(' ', ''): i * 10.0,
      };
      expect(parse(json).dark!.inputs.ansi.hues[9], 90);
    });

    final bad = <String, void Function(Map<String, Object?>)>{
      'no id': (j) => j.remove('id'),
      'a bad id': (j) => j['id'] = 'Not Valid',
      'no variants': (j) => j['variants'] = <String, Object?>{},
      'an unknown variant': (j) =>
          (j['variants']! as Map)['dim'] = <String, Object>{},
      'a missing field': (j) =>
          ((j['variants']! as Map)['light'] as Map).remove('accentH'),
      'a number out of range': (j) =>
          ((j['variants']! as Map)['light'] as Map)['accentC'] = 3,
      'a dark page in a light variant': (j) =>
          ((j['variants']! as Map)['light'] as Map)['bgL'] = 20,
      'fifteen ANSI hues': (j) =>
          ((j['variants']! as Map)['dark'] as Map)['ansiHues'] = List.filled(
            15,
            1.0,
          ),
      'a string for a number': (j) =>
          ((j['variants']! as Map)['dark'] as Map)['hue'] = 'blue',
      'a bad shape': (j) => j['shape'] = 'round',
      'an incomplete credit': (j) => j['inspiredBy'] = {'name': 'X'},
    };
    for (final MapEntry(key: name, value: spoil) in bad.entries) {
      test('$name is refused', () {
        final json = config();
        spoil(json);
        expect(() => parse(json), throwsA(isA<ThemeConfigError>()));
      });
    }

    test('text that is not JSON is refused', () {
      expect(
        () => parseThemeFamily('{', source: 'x'),
        throwsA(isA<ThemeConfigError>()),
      );
    });

    test('a bad file is skipped and the built-ins stay', () async {
      final bundle = _Bundle({
        'assets/themes/a.json': jsonEncode(config()),
        'assets/themes/storm.json': '{"id": "storm"}',
      });
      await ThemeLibrary.load(bundle);
      final ids = ThemeLibrary.families.value.map((f) => f.id).toList();
      expect(ids, ['storm', 'slowflow', 'test']);
      expect(ThemeLibrary.family('storm').dark, StormPreset.stormDark);
    });
  });

  group("Storm's own themes", () {
    test('the bundled configs are byte-identical to the built-ins', () async {
      final families = await loadAssets();
      final storm = families.firstWhere((f) => f.id == 'storm');
      final slowflow = families.firstWhere((f) => f.id == 'slowflow');
      for (final (loaded, builtIn) in [
        (storm.light!, StormPreset.stormLight),
        (storm.dark!, StormPreset.stormDark),
        (slowflow.light!, StormPreset.slowflowEarth),
      ]) {
        expect(loaded.inputs, builtIn.inputs, reason: builtIn.label);
        expect(loaded.label, builtIn.label);
        final a = StormTokens.from(loaded);
        final b = StormTokens.from(builtIn);
        expect(colours(a), colours(b), reason: builtIn.label);
        expect(a.terminal, b.terminal);
        expect(a.rCard, b.rCard);
        expect(a.shadowDepth, b.shadowDepth);
        expect(a.duration, b.duration);
      }
      expect(slowflow.dark, isNull);
    });
  });

  group('every loaded variant passes the audit', () {
    test('every pair, the terminal included', () async {
      for (final f in await loadAssets()) {
        for (final v in [f.light, f.dark].nonNulls) {
          final low = lowContrast(StormTokens.from(v));
          expect(low, isEmpty, reason: '${v.label}: $low');
        }
      }
    });

    test("a dark theme's brights read brighter than 6:1", () async {
      for (final f in await loadAssets()) {
        final v = f.dark;
        if (v == null) continue;
        final p = StormTokens.from(v).terminal;
        for (var i = 9; i < 15; i++) {
          expect(
            contrastRatio(p.ansi[i], p.background),
            greaterThanOrEqualTo(6),
            reason: '${v.label} ${AnsiInputs.names[i]}',
          );
        }
      }
    });

    test("Storm light's ANSI keeps colour at the floor (Q8)", () {
      final p = StormTokens.from(StormPreset.stormLight).terminal;
      for (final i in [2, 3, 6]) {
        expect(toOklch(p.ansi[i]).c, greaterThan(0.08), reason: '$i');
      }
    });

    test('the audit reports what fails, in plain words', () {
      final json = config();
      ((json['variants']! as Map)['dark'] as Map)
        ..['bgL'] = 40
        ..['accentL'] = 50;
      final low = lowContrast(StormTokens.from(parse(json).dark!));
      expect(low.map((c) => c.part), contains('muted text'));
      expect(low.every((c) => !c.hard), isTrue);
    });
  });

  group('the setting', () {
    test('old preset names migrate; anything unknown is Storm, system', () {
      expect(
        ThemeChoice.parse('storm-dark'),
        const ThemeChoice('storm', ThemeMode.dark),
      );
      expect(
        ThemeChoice.parse('storm-light'),
        const ThemeChoice('storm', ThemeMode.light),
      );
      expect(
        ThemeChoice.parse('slowflow-earth'),
        const ThemeChoice('slowflow', ThemeMode.light),
      );
      const system = ThemeChoice('storm', ThemeMode.system);
      for (final junk in ['retired-theme', '', 'storm/dim', '/dark', 'a/b/c']) {
        expect(ThemeChoice.parse(junk), system, reason: junk);
      }
      for (final c in [
        const ThemeChoice('rosepine', ThemeMode.system),
        const ThemeChoice('storm', ThemeMode.light),
      ]) {
        expect(ThemeChoice.parse(c.wire), c);
      }
    });

    Future<Settings> load(Map<String, Object> prefs) async {
      SharedPreferences.setMockInitialValues(prefs);
      final c = ProviderContainer();
      addTearDown(c.dispose);
      return c.read(settingsProvider.future);
    }

    test(
      'a stored preset migrates on load, and saves as family/mode',
      () async {
        SharedPreferences.setMockInitialValues({
          'storm.theme': 'slowflow-earth',
        });
        final c = ProviderContainer();
        addTearDown(c.dispose);
        final s = await c.read(settingsProvider.future);
        expect(s.theme, const ThemeChoice('slowflow', ThemeMode.light));
        await c
            .read(settingsProvider.notifier)
            .save(
              s.copyWith(theme: const ThemeChoice('storm', ThemeMode.system)),
            );
        final prefs = await SharedPreferences.getInstance();
        expect(prefs.getString('storm.theme'), 'storm/system');
      },
    );

    test('the oldest dark-mode flag still migrates', () async {
      expect(
        (await load({'storm.darkMode': false})).theme,
        const ThemeChoice('storm', ThemeMode.light),
      );
      expect((await load({})).theme, ThemeChoice.initial);
    });

    test('a family without the mode gives the member it has', () {
      const c = ThemeChoice('slowflow', ThemeMode.dark);
      expect(c.resolve(Brightness.dark), StormPreset.slowflowEarth);
      expect(
        const ThemeChoice('gone', ThemeMode.light).resolve(Brightness.dark),
        StormPreset.stormLight,
      );
    });

    testWidgets('System follows the platform brightness live', (tester) async {
      Brightness? seen;
      Widget app(ThemeChoice choice) {
        final themes = StormTheme.forChoice(choice);
        return MaterialApp(
          theme: themes.light,
          darkTheme: themes.dark,
          themeMode: choice.mode,
          home: Builder(
            builder: (context) {
              seen = context.tokens.brightness;
              return const SizedBox();
            },
          ),
        );
      }

      const system = ThemeChoice('storm', ThemeMode.system);
      tester.platformDispatcher.platformBrightnessTestValue = Brightness.light;
      addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
      await tester.pumpWidget(app(system));
      expect(seen, Brightness.light);

      tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
      await tester.pumpAndSettle();
      expect(seen, Brightness.dark);

      await tester.pumpWidget(app(const ThemeChoice('storm', ThemeMode.light)));
      await tester.pumpAndSettle();
      expect(seen, Brightness.light, reason: 'an explicit mode wins');
    });
  });

  group('the terminal', () {
    final family = parse(config());
    final light = StormTokens.from(family.light!);
    final dark = StormTokens.from(family.dark!);
    final stormDark = StormTokens.from(StormPreset.stormDark);

    Future<TerminalView> show(
      WidgetTester tester,
      ThemeVariant variant, {
      TerminalPrefs prefs = const TerminalPrefs(),
      bool shell = false,
    }) async {
      final t = StormTerminal();
      addTearDown(t.dispose);
      await tester.pumpWidget(
        MaterialApp(
          theme: StormTheme.from(variant),
          home: Scaffold(
            body: StormTerminalView(terminal: t, shell: shell, prefs: prefs),
          ),
        ),
      );
      await tester.pumpAndSettle();
      return tester.widget<TerminalView>(find.byType(TerminalView));
    }

    testWidgets("a view wears its own theme's palette", (tester) async {
      ThemeLibrary.families.value = ThemeLibrary.merge([family]);
      final v = await show(tester, family.light!, shell: true);
      expect(v.theme.background, light.terminal.background);
      expect(v.theme.blue, light.terminal.ansi[4]);
    });

    testWidgets("a dark view in a light app wears the family's dark", (
      tester,
    ) async {
      ThemeLibrary.families.value = ThemeLibrary.merge([family]);
      final v = await show(tester, family.light!);
      expect(v.theme.background, dark.terminal.background);
      expect(v.theme.red, dark.terminal.ansi[1]);
    });

    testWidgets("Always dark in a light-only family is Storm dark's", (
      tester,
    ) async {
      final v = await show(
        tester,
        StormPreset.slowflowEarth,
        shell: true,
        prefs: const TerminalPrefs(colours: TerminalColours.alwaysDark),
      );
      expect(v.theme.background, stormDark.terminal.background);
      expect(v.theme.green, stormDark.terminal.ansi[2]);
    });

    testWidgets('one theme instance per palette', (tester) async {
      ThemeLibrary.families.value = ThemeLibrary.merge([family]);
      final first = (await show(tester, family.light!, shell: true)).theme;
      final again = (await show(tester, family.light!, shell: true)).theme;
      expect(identical(first, again), isTrue);
    });
  });
}

class _Bundle extends CachingAssetBundle {
  _Bundle(this.files);

  final Map<String, String> files;

  @override
  Future<ByteData> load(String key) async {
    if (key == 'AssetManifest.bin') {
      final manifest = {
        for (final k in files.keys)
          k: [
            {'asset': k},
          ],
      };
      return const StandardMessageCodec().encodeMessage(manifest)!;
    }
    final text = files[key];
    if (text == null) throw FlutterError('no $key');
    return ByteData.sublistView(Uint8List.fromList(utf8.encode(text)));
  }
}
