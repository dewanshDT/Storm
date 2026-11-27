import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: KindBadge, path: _path)
Widget kindBadgeDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: KindBadge(
    icon: LucideIcons.circle_question_mark,
    color: context.tokens.accent,
    size: context.knobs.doubleOrNull.slider(
      label: 'size',
      initialValue: 32,
      min: 16,
      max: 96,
    ),
  ),
);

@widgetbook.UseCase(name: 'every kind', type: KindBadge, path: _path)
Widget kindBadgeKinds(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: Wrap(
      spacing: 12,
      runSpacing: 12,
      children: [
        KindBadge(icon: LucideIcons.circle_question_mark, color: t.accent),
        KindBadge(icon: LucideIcons.circle_alert, color: t.danger),
        KindBadge(icon: LucideIcons.circle_check_big, color: t.green),
        KindBadge(icon: LucideIcons.trash_2, color: t.amber),
        KindBadge(icon: LucideIcons.server_off, color: t.text3),
      ],
    ),
  );
}

@widgetbook.UseCase(name: 'custom glyph', type: KindBadge, path: _path)
Widget kindBadgeGlyph(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: KindBadge(
    glyph: Text(
      'C',
      style: TextStyle(
        color: context.tokens.accent,
        fontWeight: FontWeight.w700,
      ),
    ),
    color: context.tokens.accent,
  ),
);
