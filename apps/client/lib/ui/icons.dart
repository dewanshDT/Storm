/// The design's own glyphs, drawn rather than borrowed.
///
/// Material's set is close but not the same shape language: its folder is a
/// tab on a rounded slab, its "link" is a chain, and there is no two-ovals
/// mentions mark at all — `LucideIcons.link_2` is a node graph, which says "network"
/// where the design says "these two notes are joined". Five shapes is few
/// enough to draw, and drawing them is the only way the nav pill matches the
/// prototype.
library;

import 'package:flutter/material.dart';

enum StormGlyph { folder, newFolder, search, mentions, hash, plus, server }

class StormIcon extends StatelessWidget {
  const StormIcon(this.glyph, {super.key, this.size = 20, this.color});

  final StormGlyph glyph;
  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final paint =
        color ?? IconTheme.of(context).color ?? const Color(0xFF000000);
    return SizedBox(
      width: size,
      height: size,
      child: CustomPaint(painter: _GlyphPainter(glyph, paint)),
    );
  }
}

class _GlyphPainter extends CustomPainter {
  _GlyphPainter(this.glyph, this.color);

  final StormGlyph glyph;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    // Everything is expressed against a 20×20 box and scaled, so one set of
    // proportions holds at every size the tokens ask for.
    final k = size.width / 20;
    final fill = Paint()..color = color;
    final stroke = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.7 * k
      ..strokeCap = StrokeCap.round;

    // A tab, then the body — the design's folder is two rectangles, not a
    // single shape with a notch.
    void folder({double width = 16, double top = 4}) {
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(2 * k, top * k, width * 0.44 * k, 2.5 * k),
          Radius.circular(1 * k),
        ),
        fill,
      );
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(2 * k, (top + 1.6) * k, width * k, 10 * k),
          Radius.circular(1.6 * k),
        ),
        fill,
      );
    }

    switch (glyph) {
      case StormGlyph.folder:
        folder();

      case StormGlyph.newFolder:
        // A narrower folder, with the `+` in the clear space beside it rather
        // than punched out of the body — a punch needs to know the ground it
        // is drawn on, and this glyph appears on three different ones.
        folder(width: 12, top: 6);
        final centre = Offset(16 * k, 6.5 * k);
        canvas.drawLine(
          centre - Offset(2.6 * k, 0),
          centre + Offset(2.6 * k, 0),
          stroke,
        );
        canvas.drawLine(
          centre - Offset(0, 2.6 * k),
          centre + Offset(0, 2.6 * k),
          stroke,
        );

      case StormGlyph.search:
        canvas.drawCircle(Offset(8.5 * k, 8.5 * k), 5.5 * k, stroke);
        canvas.drawLine(
          Offset(12.8 * k, 12.8 * k),
          Offset(17 * k, 17 * k),
          stroke,
        );

      case StormGlyph.mentions:
        // Two rounded rectangles, overlapping — a link, as the design draws
        // it, rather than Material's chain.
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTWH(1.5 * k, 6.5 * k, 10 * k, 7 * k),
            Radius.circular(3.5 * k),
          ),
          stroke,
        );
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTWH(8.5 * k, 6.5 * k, 10 * k, 7 * k),
            Radius.circular(3.5 * k),
          ),
          stroke,
        );

      case StormGlyph.hash:
        final thin = Paint()
          ..color = color
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.9 * k
          ..strokeCap = StrokeCap.round;
        canvas.drawLine(Offset(7.5 * k, 3 * k), Offset(5.8 * k, 17 * k), thin);
        canvas.drawLine(Offset(14 * k, 3 * k), Offset(12.3 * k, 17 * k), thin);
        canvas.drawLine(
          Offset(3.5 * k, 7.5 * k),
          Offset(17 * k, 7.5 * k),
          thin,
        );
        canvas.drawLine(
          Offset(2.8 * k, 12.8 * k),
          Offset(16.3 * k, 12.8 * k),
          thin,
        );

      case StormGlyph.server:
        // Two stacked units with a status lamp each — a machine, not a
        // directory. The dashboard's second slot opens server settings, and
        // it was wearing the folder glyph.
        for (final top in [3.0, 11.0]) {
          canvas.drawRRect(
            RRect.fromRectAndRadius(
              Rect.fromLTWH(2 * k, top * k, 16 * k, 6 * k),
              Radius.circular(1.5 * k),
            ),
            stroke,
          );
          canvas.drawCircle(Offset(5.5 * k, (top + 3) * k), 0.9 * k, fill);
        }

      case StormGlyph.plus:
        final thick = Paint()
          ..color = color
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2.1 * k
          ..strokeCap = StrokeCap.round;
        canvas.drawLine(Offset(10 * k, 4 * k), Offset(10 * k, 16 * k), thick);
        canvas.drawLine(Offset(4 * k, 10 * k), Offset(16 * k, 10 * k), thick);
    }
  }

  @override
  bool shouldRepaint(_GlyphPainter old) =>
      old.glyph != glyph || old.color != color;
}
