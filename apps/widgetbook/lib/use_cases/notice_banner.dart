import 'package:flutter/material.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

@widgetbook.UseCase(name: 'default', type: NoticeBanner, path: _path)
Widget noticeBannerDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: NoticeBanner(
    child: Text(
      context.knobs.string(
        label: 'text',
        initialValue: 'Turn on agents starting agents?',
      ),
    ),
  ),
);

@widgetbook.UseCase(name: 'warning and strong', type: NoticeBanner, path: _path)
Widget noticeBannerTones(BuildContext context) {
  final t = context.tokens;
  return Padding(
    padding: const EdgeInsets.all(16),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        NoticeBanner(
          color: t.amberSoft,
          borderColor: t.amber,
          child: const Text('Expires in 30 days.'),
        ),
        const SizedBox(height: 8),
        NoticeBanner(
          color: t.surface,
          borderColor: t.danger,
          borderWidth: t.bw * 2,
          child: const Text('Expires in 7 days.'),
        ),
      ],
    ),
  );
}
