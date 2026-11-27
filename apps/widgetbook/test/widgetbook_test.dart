import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/ui/themes/theme_library.dart';
import 'package:storm_widgetbook/main.dart';
import 'package:storm_widgetbook/main.directories.g.dart';
import 'package:storm_widgetbook/storm_addons.dart';
import 'package:widgetbook/widgetbook.dart';

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

  group('every use case renders', () {
    final useCases = _useCases(directories);

    test('there are some', () => expect(useCases, isNotEmpty));

    for (final useCase in useCases) {
      testWidgets(useCase.path, (tester) async {
        tester.view.physicalSize = const Size(1600, 1000);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        await tester.pumpWidget(
          Widgetbook.material(
            directories: directories,
            addons: stormAddons(),
            initialRoute: Uri(
              path: '/',
              queryParameters: {'path': useCase.path, 'preview': ''},
            ).toString(),
          ),
        );
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        expect(find.byType(Widgetbook), findsOneWidget);
      });
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
