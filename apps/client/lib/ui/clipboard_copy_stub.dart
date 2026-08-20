import 'package:flutter/services.dart';

/// Off the web the platform clipboard is always there and always allowed.
///
/// `false` only for a genuine platform-channel failure, which the caller
/// reports rather than swallows.
Future<bool> copyToClipboard(String text) async {
  try {
    await Clipboard.setData(ClipboardData(text: text));
    return true;
  } catch (_) {
    return false;
  }
}
