import 'image_paste.dart';

/// Native platforms read the clipboard on the paste key instead.
void Function() listenForPastedImages(
  void Function(List<PastedImage>) onImages,
) => () {};
