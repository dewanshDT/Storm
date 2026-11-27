import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';
const _long = 'a-very-long-vault-name-that-cannot-fit-in-its-tag-at-any-width';

@widgetbook.UseCase(name: 'default', type: OutlineTag, path: _path)
Widget outlineTagDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: OutlineTag(
    label: context.knobs.string(label: 'label', initialValue: 'Personal'),
    tone: context.knobs.object.dropdown<StormTone>(
      label: 'tone',
      options: StormTone.values,
      initialOption: StormTone.neutral,
      labelBuilder: (v) => v.name,
    ),
    shape: context.knobs.object.dropdown<OutlineTagShape>(
      label: 'shape',
      options: OutlineTagShape.values,
      initialOption: OutlineTagShape.pill,
      labelBuilder: (v) => v.name,
    ),
    mono: context.knobs.boolean(label: 'mono', initialValue: true),
    borderColor: context.knobs.boolean(label: 'border', initialValue: true)
        ? context.tokens.border
        : null,
  ),
);

@widgetbook.UseCase(name: 'tones', type: OutlineTag, path: _path)
Widget outlineTagTones(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: Wrap(
      spacing: 8,
      runSpacing: 8,
      children: [
        for (final tone in StormTone.values)
          OutlineTag(label: tone.name, tone: tone, borderColor: t.border),
      ],
    ),
  );
}

@widgetbook.UseCase(name: 'shapes and fill', type: OutlineTag, path: _path)
Widget outlineTagShapes(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: Wrap(
      spacing: 8,
      runSpacing: 8,
      children: [
        OutlineTag(label: 'pill', borderColor: t.border),
        OutlineTag(
          label: 'tag',
          borderColor: t.border,
          shape: OutlineTagShape.tag,
        ),
        OutlineTag(label: 'filled', color: t.accent, fill: t.accentSoft),
        OutlineTag(label: 'sans', borderColor: t.border, mono: false),
      ],
    ),
  );
}

@widgetbook.UseCase(name: 'leading and trailing', type: OutlineTag, path: _path)
Widget outlineTagAdorned(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: Wrap(
      spacing: 8,
      runSpacing: 8,
      children: [
        OutlineTag(
          label: 'Running',
          color: t.accent,
          borderColor: t.border,
          leading: Icon(Icons.circle, size: 7, color: t.accent),
        ),
        OutlineTag(
          label: 'removable',
          borderColor: t.border,
          trailing: Icon(Icons.close, size: 12, color: t.text3),
          onTap: () {},
        ),
      ],
    ),
  );
}

@widgetbook.UseCase(name: 'tappable', type: OutlineTag, path: _path)
Widget outlineTagTappable(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: OutlineTag(
    label: 'tap me',
    borderColor: context.tokens.border,
    onTap: () {},
  ),
);

@widgetbook.UseCase(name: 'focused', type: OutlineTag, path: _path)
Widget outlineTagFocused(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Focus(
    autofocus: true,
    child: OutlineTag(
      label: 'focused',
      borderColor: context.tokens.border,
      onTap: () {},
    ),
  ),
);

@widgetbook.UseCase(
  name: 'long label (overflow)',
  type: OutlineTag,
  path: _path,
)
Widget outlineTagLong(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      SizedBox(
        width: 160,
        child: OutlineTag(
          label: context.knobs.string(label: 'label', initialValue: _long),
          borderColor: context.tokens.border,
        ),
      ),
      const SizedBox(height: 8),
      OutlineTag(
        label: _long,
        borderColor: context.tokens.border,
        maxWidth: 140,
        minWidth: 60,
      ),
    ],
  ),
);
