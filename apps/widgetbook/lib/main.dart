import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

import 'main.directories.g.dart';
import 'storm_addons.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await loadStormThemes(rootBundle);
  runApp(const StormWidgetbook());
}

@widgetbook.App()
class StormWidgetbook extends StatelessWidget {
  const StormWidgetbook({super.key});

  @override
  Widget build(BuildContext context) {
    return Widgetbook.material(directories: directories, addons: stormAddons());
  }
}
