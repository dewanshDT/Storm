import 'package:alchemist/alchemist.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/themes/theme_library.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm_widgetbook/main.directories.g.dart';
import 'package:storm_widgetbook/storm_addons.dart';
import 'package:widgetbook/widgetbook.dart';

const _themes = ['Storm dark', 'Storm light', 'SlowFlow earth'];
const _widths = {'phone': 390.0, 'desktop': 1280.0};

void main() {
  late List<(String, ThemeData)> themes;
  final state = WidgetbookState(
    root: WidgetbookRoot(children: directories),
    previewMode: true,
  );

  setUpAll(() async {
    TestWidgetsFlutterBinding.ensureInitialized();
    await loadStormThemes(rootBundle);
    final variants = {
      for (final family in ThemeLibrary.families.value)
        for (final v in [family.dark, family.light].whereType<ThemeVariant>())
          v.label: StormTheme.from(v),
    };
    themes = [for (final name in _themes) (name, variants[name]!)];
  });

  for (final component in _components(directories)) {
    goldenTest(
      component.path,
      fileName: _fileName(component.path),
      tags: ['golden'],
      pumpWidget: (tester, widget) {
        _ignoreOverflow();
        return tester.pumpWidget(
          Localizations(
            locale: const Locale('en'),
            delegates: const [
              DefaultWidgetsLocalizations.delegate,
              DefaultMaterialLocalizations.delegate,
            ],
            child: widget,
          ),
        );
      },
      pumpBeforeTest: (tester) async {
        await precacheImages(tester);
        await tester.pump(const Duration(milliseconds: 100));
      },
      builder: () => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final width in _widths.entries)
            for (final useCase in component.children!.cast<WidgetbookUseCase>())
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  for (final (name, theme) in themes)
                    GoldenTestScenario(
                      name: '${useCase.name} / $name / ${width.key}',
                      child: _Cell(
                        width: width.value,
                        theme: theme,
                        state: state,
                        useCase: useCase,
                      ),
                    ),
                ],
              ),
        ],
      ),
    );
  }
}

/// The window size and the widgetbook state a use case reads. A nested
/// MaterialApp would fill the cell's height, so it is not used.
class _Cell extends StatelessWidget {
  const _Cell({
    required this.width,
    required this.theme,
    required this.state,
    required this.useCase,
  });

  final double width;
  final ThemeData theme;
  final WidgetbookState state;
  final WidgetbookUseCase useCase;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: width,
      child: MediaQuery(
        data: MediaQuery.of(context).copyWith(size: Size(width, 844)),
        child: TickerMode(
          enabled: false,
          child: WidgetbookScope(
            state: state,
            child: stormThemed(theme, Builder(builder: useCase.build)),
          ),
        ),
      ),
    );
  }
}

/// CI goldens draw text in a font whose glyphs are far wider than the real
/// ones, so a row that fits in production overflows here; the stripe it paints
/// still shows in the image, but it is not a failure.
void _ignoreOverflow() {
  final previous = FlutterError.onError;
  FlutterError.onError = (details) {
    if (details.exception.toString().contains('overflowed')) return;
    previous?.call(details);
  };
  addTearDown(() => FlutterError.onError = previous);
}

String _fileName(String path) =>
    path.replaceAll(RegExp(r'[^A-Za-z0-9]+'), '_').toLowerCase();

// The index lists file:line sites in the client, so its image would change
// with every client edit; the smoke test still renders it.
List<WidgetbookNode> _components(List<WidgetbookNode> nodes) => [
  for (final node in nodes)
    if (node is WidgetbookComponent)
      node
    else if (node.children != null && node.name != 'Index')
      ..._components(node.children!),
];
