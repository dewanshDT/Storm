import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';

/// Client build stamp from the Flutter package (`version` + optional build).
///
/// Release CI passes `--build-name` / `--build-number` from the git tag; local
/// `flutter run` shows pubspec's placeholder (`1.0.0+1`). Shown in Client
/// settings so a stale web service-worker cache is obvious without guessing.
final clientVersionProvider = FutureProvider<String>((ref) async {
  final info = await PackageInfo.fromPlatform();
  final build = info.buildNumber.trim();
  if (build.isEmpty) return info.version;
  return '${info.version}+$build';
});
