import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';

import '../tokens.dart';

/// Storm-styled [MarkdownStyleSheet] for Read Mode.
///
/// Tuned against `docs/ss html/prototype-*-note.png` and the note body in
/// `Storm.dc.html`: Newsreader prose, compact lists, section rhythm from
/// heading margins — not from a large [MarkdownStyleSheet.blockSpacing]
/// (that gap is inserted between every `li`, which blew list spacing apart).
MarkdownStyleSheet stormMarkdownStyleSheet({
  required BuildContext context,
  required String? bodyFamily,
  required double fontSize,
}) {
  final t = context.tokens;

  // Prototype phone: 16 / 1.65; wide: 17 / 1.7. Keep the user's size, match the
  // leading and the softer prose colour.
  final body = TextStyle(
    fontFamily: bodyFamily,
    fontSize: fontSize,
    height: 1.65,
    color: t.text2,
  );

  TextStyle heading(double size, {FontWeight weight = FontWeight.w600}) =>
      TextStyle(
        fontFamily: bodyFamily ?? StormTokens.serifFamily,
        fontSize: size,
        height: 1.25,
        fontWeight: weight,
        color: t.text,
      );

  final mono = TextStyle(
    fontFamily: StormTokens.monoFamily,
    fontSize: fontSize * 0.84,
    height: 1.45,
    color: t.text,
  );

  // Checkbox column in the prototype is an 18px box + 10px gap. Regular
  // bullets share the same indent so lists line up.
  const checkboxSide = kStormMarkdownCheckboxSize;
  const checkboxGap = 10.0;

  return MarkdownStyleSheet(
    a: body.copyWith(
      color: t.accent,
      fontWeight: FontWeight.w500,
      decoration: TextDecoration.underline,
      decorationColor: t.accent.withValues(alpha: 0.45),
    ),
    p: body,
    // Modest bottom pad; section gaps come from heading top margins.
    // blockSpacing is *also* inserted between siblings — keep both small.
    pPadding: EdgeInsets.only(bottom: t.sp * 0.75),
    h1: heading(fontSize * t.scale * t.scale),
    h1Padding: EdgeInsets.only(top: t.sp * 0.5, bottom: t.sp * 1.75),
    h2: heading(fontSize * t.scale),
    h2Padding: EdgeInsets.only(top: t.sp * 2.25, bottom: t.sp),
    h3: heading(fontSize, weight: FontWeight.w600),
    h3Padding: EdgeInsets.only(top: t.sp * 1.75, bottom: t.sp * 0.75),
    h4: heading(fontSize, weight: FontWeight.w500),
    h4Padding: EdgeInsets.only(top: t.sp * 1.5, bottom: t.sp * 0.5),
    h5: body.copyWith(fontWeight: FontWeight.w600, color: t.text),
    h5Padding: EdgeInsets.only(top: t.sp, bottom: t.sp * 0.5),
    h6: body.copyWith(fontWeight: FontWeight.w500, color: t.text2),
    h6Padding: EdgeInsets.only(top: t.sp, bottom: t.sp * 0.5),
    em: body.copyWith(fontStyle: FontStyle.italic),
    strong: body.copyWith(fontWeight: FontWeight.w700, color: t.text),
    del: body.copyWith(
      decoration: TextDecoration.lineThrough,
      // Prototype: `opacity:0.6` on completed task labels.
      color: t.text2.withValues(alpha: 0.6),
    ),
    code: mono.copyWith(backgroundColor: t.surface2),
    blockquote: body.copyWith(fontStyle: FontStyle.italic),
    blockquotePadding: EdgeInsets.only(left: t.sp * 1.75),
    blockquoteDecoration: BoxDecoration(
      border: Border(left: BorderSide(color: t.accent, width: 3)),
    ),
    codeblockPadding: EdgeInsets.symmetric(
      horizontal: t.sp * 1.75,
      vertical: t.sp * 1.5,
    ),
    codeblockDecoration: BoxDecoration(
      color: t.surface,
      borderRadius: BorderRadius.circular(t.rControl),
      border: Border.all(color: t.border, width: t.bw),
    ),
    listIndent: checkboxSide,
    listBullet: body.copyWith(color: t.text2),
    listBulletPadding: const EdgeInsets.only(right: checkboxGap),
    checkbox: body.copyWith(color: t.accent),
    // Inserted between *every* sibling, including list items. Prototype
    // checklists use an 8px gap; bullets rely on line-height alone.
    blockSpacing: t.sp,
    tableHead: body.copyWith(fontWeight: FontWeight.w600, color: t.text),
    tableBody: body,
    tableHeadAlign: TextAlign.left,
    tablePadding: EdgeInsets.symmetric(vertical: t.sp),
    tableBorder: TableBorder.all(color: t.border, width: t.bw),
    tableColumnWidth: const IntrinsicColumnWidth(),
    tableCellsPadding: EdgeInsets.symmetric(
      horizontal: t.sp * 1.25,
      vertical: t.sp * 0.75,
    ),
    tableHeadCellsDecoration: BoxDecoration(color: t.surface2),
    tableCellsDecoration: const BoxDecoration(),
    horizontalRuleDecoration: BoxDecoration(
      border: Border(
        top: BorderSide(color: t.border, width: t.bw),
      ),
    ),
    img: body,
  );
}

/// Prototype checkbox is a fixed 18×18 rounded square (`Storm.dc.html` boxStyle).
const kStormMarkdownCheckboxSize = 18.0;
