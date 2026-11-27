/// Brand mark, image thumbnails and code blocks.
library;

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import 'clipboard_copy.dart';
import 'controls.dart' show StormButton;
import 'tappable.dart';
import 'tokens.dart';

/// The product mark.
///
/// **The mint ground is fixed brand colour and does not re-theme.** It is the
/// one surface in the app the token layer deliberately cannot reach: a mark
/// that changes colour with the theme stops being a mark. The asset already
/// ships for the launcher icon, so this is the same image the OS shows.
class BrandMark extends StatelessWidget {
  const BrandMark({super.key, this.size = 28, this.withWordmark = false});

  final double size;
  final bool withWordmark;

  /// Fixed. Not a token, and not derived.
  static const groundColor = Color(0xFF96F2D7);

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final mark = ClipRRect(
      borderRadius: BorderRadius.circular(size * 0.28),
      child: Image.asset(
        'assets/icon/storm_icon_full.png',
        width: size,
        height: size,
        fit: BoxFit.cover,
      ),
    );
    if (!withWordmark) return mark;

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        mark,
        SizedBox(width: t.sp),
        Text(
          'STORM',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.labelSize,
            fontWeight: FontWeight.w600,
            letterSpacing: 1.6,
            color: t.text2,
          ),
        ),
      ],
    );
  }
}

/// A square image preview with a remove button in its corner — a screenshot
/// attached to a report (decision 162a), or any other image chosen before
/// it's confirmed.
class ImageThumbnail extends StatelessWidget {
  const ImageThumbnail({
    super.key,
    required this.image,
    required this.onRemove,
  });

  final ImageProvider image;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = t.sp * 11;
    return SizedBox(
      width: side,
      height: side,
      child: Stack(
        children: [
          ClipRRect(
            borderRadius: BorderRadius.circular(t.rControl),
            child: Image(
              image: image,
              width: side,
              height: side,
              fit: BoxFit.cover,
              errorBuilder: (_, _, _) => ColoredBox(
                color: t.surface2,
                child: Center(
                  child: Icon(
                    LucideIcons.image_off,
                    size: side * 0.4,
                    color: t.text3,
                  ),
                ),
              ),
            ),
          ),
          Positioned(
            top: t.sp * 0.5,
            right: t.sp * 0.5,
            child: Material(
              color: t.surface.withValues(alpha: 0.85),
              shape: const CircleBorder(),
              child: Tappable(
                semanticLabel: 'Remove',
                onTap: onRemove,
                borderRadius: BorderRadius.circular(t.sp * 2),
                child: Padding(
                  padding: EdgeInsets.all(t.sp * 0.375),
                  child: Icon(
                    LucideIcons.x,
                    size: t.labelSize,
                    color: t.text,
                    semanticLabel: 'Remove',
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// A block of monospace text: `surface2`, bordered, scrollable, selectable,
/// with an optional "Copy" action — the shape that kept appearing hand-rolled
/// for anything code- or log-shaped (a JSON preview, a diagnostics dump).
class CodeBlock extends StatelessWidget {
  const CodeBlock({
    super.key,
    required this.text,
    this.maxHeight,
    this.showCopy = false,
    this.onCopied,
    this.copyButtonKey,
  });

  final String text;
  final double? maxHeight;

  /// Shows a "Copy" action above the block, wired to [onCopied].
  final bool showCopy;

  /// Called after a copy attempt — `clipboard_copy.dart`'s own semantics:
  /// `false` only for a genuine platform failure. The caller decides how to
  /// say so, usually a snackbar.
  final ValueChanged<bool>? onCopied;
  final Key? copyButtonKey;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final panel = Container(
      padding: EdgeInsets.all(t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: SingleChildScrollView(
        child: SelectableText(
          text,
          style: TextStyle(
            fontFamily: StormTokens.monoFamily,
            fontSize: t.labelSize * 1.09,
            color: t.text,
          ),
        ),
      ),
    );
    final bounded = maxHeight == null
        ? panel
        : ConstrainedBox(
            constraints: BoxConstraints(maxHeight: maxHeight!),
            child: panel,
          );
    if (!showCopy) return bounded;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Align(
          alignment: Alignment.centerRight,
          child: StormButton.text(
            key: copyButtonKey,
            label: 'Copy',
            onPressed: () =>
                copyToClipboard(text).then((ok) => onCopied?.call(ok)),
          ),
        ),
        SizedBox(height: t.sp),
        bounded,
      ],
    );
  }
}
