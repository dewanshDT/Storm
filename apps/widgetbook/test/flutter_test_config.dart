import 'dart:async';

import 'package:alchemist/alchemist.dart';

/// Only the CI goldens (text drawn as blocks, no platform fonts) are
/// committed: they render the same on every machine, so nothing in the
/// repository depends on whose laptop made it.
Future<void> testExecutable(FutureOr<void> Function() testMain) {
  return AlchemistConfig.runWithConfig(
    config: const AlchemistConfig(
      platformGoldensConfig: PlatformGoldensConfig(enabled: false),
    ),
    run: () async => testMain(),
  );
}
