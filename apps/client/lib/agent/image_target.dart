import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/material.dart';

import '../ui/tokens.dart';
import 'image_paste.dart';
import 'paste_listener.dart'
    if (dart.library.js_interop) 'paste_listener_web.dart';

/// Takes images dropped on a session's terminal, and on the web images
/// pasted while it has focus (D15 AM45).
class ImagePasteTarget extends StatefulWidget {
  const ImagePasteTarget({
    super.key,
    required this.child,
    required this.onImages,
    required this.focused,
  });

  final Widget child;
  final void Function(List<PastedImage> images) onImages;

  /// Whether a paste is the terminal's: the browser's paste event has no
  /// other way to say where it was aimed.
  final bool Function() focused;

  @override
  State<ImagePasteTarget> createState() => _ImagePasteTargetState();
}

class _ImagePasteTargetState extends State<ImagePasteTarget> {
  late final void Function() _unlisten;
  bool _over = false;

  @override
  void initState() {
    super.initState();
    _unlisten = listenForPastedImages((images) {
      if (mounted && widget.focused()) widget.onImages(images);
    });
  }

  @override
  void dispose() {
    _unlisten();
    super.dispose();
  }

  Future<void> _dropped(DropDoneDetails d) async {
    setState(() => _over = false);
    final images = <PastedImage>[];
    for (final file in d.files) {
      final image = await normalizeImage(await file.readAsBytes());
      if (image != null) images.add(image);
    }
    if (mounted && images.isNotEmpty) widget.onImages(images);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return DropTarget(
      onDragEntered: (_) => setState(() => _over = true),
      onDragExited: (_) => setState(() => _over = false),
      onDragDone: _dropped,
      child: DecoratedBox(
        position: DecorationPosition.foreground,
        decoration: BoxDecoration(
          border: _over
              ? Border.all(color: t.accent, width: t.bw * 2)
              : const Border(),
        ),
        child: widget.child,
      ),
    );
  }
}
