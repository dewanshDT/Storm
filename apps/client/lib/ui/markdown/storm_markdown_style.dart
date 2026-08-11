import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';

import '../tokens.dart';

/// Storm-styled [MarkdownStyleSheet] for Read Mode.
///
/// Driven entirely by [StormTokens] and the note-body face the user chose —
/// no package defaults, no Material blues.
MarkdownStyleSheet stormMarkdownStyleSheet({
  required BuildContext context,
  required String? bodyFamily,
  required double fontSize,
}) {
  final t = context.tokens;

  final body = TextStyle(
    fontFamily: bodyFamily,
    fontSize: fontSize,
    height: 1.65,
    color: t.text,
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
    fontSize: fontSize / t.scale,
    height: 1.45,
    color: t.text,
  );

  return MarkdownStyleSheet(
    a: body.copyWith(
      color: t.accent,
      decoration: TextDecoration.underline,
      decorationColor: t.accent.withValues(alpha: 0.45),
    ),
    p: body,
    pPadding: EdgeInsets.only(bottom: t.sp * 0.5),
    h1: heading(fontSize * t.scale * t.scale),
    h1Padding: EdgeInsets.only(top: t.sp * 2.5, bottom: t.sp * 1.25),
    h2: heading(fontSize * t.scale),
    h2Padding: EdgeInsets.only(top: t.sp * 2.25, bottom: t.sp),
    h3: heading(fontSize, weight: FontWeight.w600),
    h3Padding: EdgeInsets.only(top: t.sp * 1.75, bottom: t.sp * 0.75),
    h4: heading(fontSize, weight: FontWeight.w500),
    h4Padding: EdgeInsets.only(top: t.sp * 1.5, bottom: t.sp * 0.5),
    h5: body.copyWith(fontWeight: FontWeight.w600),
    h5Padding: EdgeInsets.only(top: t.sp, bottom: t.sp * 0.5),
    h6: body.copyWith(fontWeight: FontWeight.w500, color: t.text2),
    h6Padding: EdgeInsets.only(top: t.sp, bottom: t.sp * 0.5),
    em: body.copyWith(fontStyle: FontStyle.italic),
    strong: body.copyWith(fontWeight: FontWeight.w700),
    del: body.copyWith(decoration: TextDecoration.lineThrough, color: t.text3),
    code: mono.copyWith(backgroundColor: t.surface2, color: t.text),
    blockquote: body.copyWith(color: t.text2, fontStyle: FontStyle.italic),
    blockquotePadding: EdgeInsets.fromLTRB(t.sp * 1.75, t.sp, t.sp, t.sp),
    blockquoteDecoration: BoxDecoration(
      border: Border(
        left: BorderSide(color: t.accent, width: t.bw * 2.5),
      ),
    ),
    codeblockPadding: EdgeInsets.all(t.sp * 1.5),
    codeblockDecoration: BoxDecoration(
      color: t.surface2,
      borderRadius: BorderRadius.circular(t.rControl),
      border: Border.all(color: t.border, width: t.bw),
    ),
    listIndent: t.sp * 3,
    listBullet: body.copyWith(color: t.text2),
    listBulletPadding: EdgeInsets.only(right: t.sp),
    checkbox: body.copyWith(color: t.accent),
    blockSpacing: t.sp * 1.5,
    tableHead: body.copyWith(fontWeight: FontWeight.w600),
    tableBody: body.copyWith(fontSize: fontSize / t.scale * 1.05),
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
