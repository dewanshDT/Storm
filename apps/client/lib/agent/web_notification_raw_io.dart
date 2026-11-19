/// The native stand-in for `web_notification_raw_web.dart` — never actually
/// reached, since every caller gates on `kIsWeb` first, but this file still
/// has to compile for every native target.
library;

bool rawIsSecureContext() => false;

String rawPermission() => 'unsupported';

Future<String> rawRequestPermission() async => 'unsupported';

void rawShow({
  required int id,
  required String title,
  String? body,
  String? icon,
  String? payload,
  required bool silent,
  required void Function(String? payload) onClick,
}) {}

void rawClose(int id) {}

void rawCloseAll() {}
