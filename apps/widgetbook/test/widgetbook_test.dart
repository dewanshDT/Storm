import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/ui/themes/theme_library.dart';
import 'package:storm_widgetbook/main.dart';
import 'package:storm_widgetbook/main.directories.g.dart';
import 'package:storm_widgetbook/storm_addons.dart';
import 'package:widgetbook/widgetbook.dart';

const _themes = ['Storm dark', 'Storm light', 'SlowFlow earth'];

void main() {
  setUpAll(() async {
    TestWidgetsFlutterBinding.ensureInitialized();
    await loadStormThemes(rootBundle);
  });

  test('every theme family loads with a theme per variant', () {
    final names = stormThemes().map((t) => t.name).toList();
    expect(ThemeLibrary.families.value.length, greaterThanOrEqualTo(8));
    expect(names.length, greaterThanOrEqualTo(15));
    expect(names.toSet().length, names.length);
  });

  testWidgets('the catalogue builds', (tester) async {
    tester.view.physicalSize = const Size(1600, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(const StormWidgetbook());
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
  });

  // The goldens ignore overflow (their block font is wider than real text),
  // so this is what catches it, with real fonts, under the goldens' themes.
  group('every use case renders', () {
    final useCases = _useCases(directories);

    test('there are some', () => expect(useCases, isNotEmpty));

    for (final label in _themes) {
      for (final useCase in useCases) {
        testWidgets('$label: ${useCase.path}', (tester) async {
          final theme = stormThemes().singleWhere(
            (t) => t.name.endsWith('($label)'),
          );
          tester.view.physicalSize = const Size(1600, 1000);
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.reset);
          await tester.pumpWidget(
            Widgetbook.material(
              directories: directories,
              addons: stormAddons(),
              initialRoute: Uri(
                path: '/',
                queryParameters: {
                  'path': useCase.path,
                  'preview': '',
                  'theme': FieldCodec.encodeQueryGroup({'name': theme.name}),
                },
              ).toString(),
            ),
          );
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          expect(
            find.byWidgetPredicate(
              (w) => w is Theme && w.data.colorScheme == theme.data.colorScheme,
            ),
            findsWidgets,
          );
        });
      }
    }
  });
}

List<WidgetbookUseCase> _useCases(List<WidgetbookNode> nodes) => [
  for (final node in nodes)
    if (node is WidgetbookUseCase)
      node
    else if (node.children != null)
      ..._useCases(node.children!),
];
