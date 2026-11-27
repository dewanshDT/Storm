import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';
const _json = '{\n  "kind": "bug",\n  "screen": "settings/health"\n}';

@widgetbook.UseCase(name: 'default', type: CodeBlock, path: _path)
Widget codeBlockDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: CodeBlock(
    text: context.knobs.string(label: 'text', initialValue: _json, maxLines: 6),
    showCopy: context.knobs.boolean(label: 'showCopy', initialValue: true),
    maxHeight: context.knobs.doubleOrNull.slider(
      label: 'maxHeight',
      initialValue: null,
      min: 40,
      max: 400,
    ),
  ),
);

@widgetbook.UseCase(name: 'long text (scrolls)', type: CodeBlock, path: _path)
Widget codeBlockLong(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: CodeBlock(
    text: [
      for (var i = 1; i <= 40; i++)
        'line $i: a long log line that carries on past the width of the block',
    ].join('\n'),
    maxHeight: 160,
    showCopy: true,
  ),
);
