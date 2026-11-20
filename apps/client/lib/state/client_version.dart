import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';

/// Client build stamp from the Flutter package (`version` + optional build).
///
/// Release CI passes `--build-name` / `--build-number` from the git tag; local
/// `flutter run` shows pubspec's placeholder (`0.1.0+1`). Shown in Client
/// settings so a stale web service-worker cache is obvious without guessing.
final clientVersionProvider = FutureProvider<String>((ref) async {
  final info = await PackageInfo.fromPlatform();
  final build = info.buildNumber.trim();
  if (build.isEmpty) return info.version;
  return '${info.version}+$build';
});

/// Decision 163: the release line this build was published from — `main`'s
/// stable (GitHub's "Latest" release) or `staging`'s latest, its pre-release
/// line.
enum StormChannel { stable, latest }

const String _stormChannelEnv = String.fromEnvironment(
  'STORM_CHANNEL',
  defaultValue: 'latest',
);

/// The release workflow's `channel` job stamps this on every app build
/// (`--dart-define=STORM_CHANNEL=stable|latest`), read nowhere else — every
/// job passes it explicitly rather than re-deriving it. Unset elsewhere (a
/// local `flutter run`, a web preview, or the Acceptance workflow, which
/// stamps `latest` itself) defaults to `latest`: a build CI didn't stamp is
/// never the one GitHub calls Latest.
const StormChannel stormChannel = _stormChannelEnv == 'stable'
    ? StormChannel.stable
    : StormChannel.latest;

/// [stormChannel], so widgets can watch it and tests can override it without
/// needing a different compiled binary.
final stormChannelProvider = Provider<StormChannel>((ref) => stormChannel);
