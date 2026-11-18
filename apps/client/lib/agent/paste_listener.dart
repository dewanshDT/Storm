import 'image_paste.dart';

/// Native platforms read the clipboard on the paste key instead.
void Function() listenForPaste({
  required bool Function() focused,
  required void Function(List<PastedImage>) onImages,
  required void Function(String) onText,
}) => () {};
