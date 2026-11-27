import 'package:flutter/material.dart';
import 'package:storm/ui/controls.dart';
import 'package:storm/ui/surfaces.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Surfaces]';

Widget _body(BuildContext context, String text) => Text(
  text,
  style: TextStyle(
    fontFamily: StormTokens.sansFamily,
    fontSize: context.tokens.uiSize,
    color: context.tokens.text,
  ),
);

Widget _frame(Widget child) => Align(
  alignment: Alignment.topCenter,
  child: ConstrainedBox(
    constraints: const BoxConstraints(maxWidth: 420, maxHeight: 520),
    child: Padding(padding: const EdgeInsets.all(16), child: child),
  ),
);

@widgetbook.UseCase(name: 'default', type: StormSheet, path: _path)
Widget stormSheetDefault(BuildContext context) => _frame(
  StormSheet(
    title: context.knobs.string(label: 'title', initialValue: 'Properties'),
    scrollable: context.knobs.boolean(label: 'scrollable', initialValue: true),
    onClose: context.knobs.boolean(label: 'closable', initialValue: true)
        ? () {}
        : null,
    child: _body(
      context,
      context.knobs.string(
        label: 'body',
        initialValue:
            'A bottom sheet: surface, rCard top corners, a labelled header.',
        maxLines: 4,
      ),
    ),
  ),
);

@widgetbook.UseCase(name: 'no close affordance', type: StormSheet, path: _path)
Widget stormSheetNoClose(BuildContext context) => _frame(
  StormSheet(title: 'Properties', child: _body(context, 'No onClose.')),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: StormSheet, path: _path)
Widget stormSheetLong(BuildContext context) => _frame(
  SizedBox(
    height: 240,
    child: StormSheet(
      title: 'A title that is far too long for a sheet header to hold',
      onClose: () {},
      child: _body(context, List.filled(30, 'Scrolling body text.').join(' ')),
    ),
  ),
);

@widgetbook.UseCase(
  name: 'adaptive sheet (opens)',
  type: StormSheet,
  path: _path,
)
Widget stormSheetAdaptive(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: Align(
    alignment: Alignment.topLeft,
    child: StormButton.outline(
      label: 'Open demo sheet',
      onPressed: () => showAdaptiveSheet<void>(
        context: context,
        title: 'Adaptive sheet',
        builder: (_) => _body(
          context,
          'A dialog at desk width, a bottom sheet on a phone.',
        ),
      ),
    ),
  ),
);

@widgetbook.UseCase(name: 'default', type: SheetHost, path: _path)
Widget sheetHostDefault(BuildContext context) => SheetHost(
  title: context.knobs.string(label: 'title', initialValue: 'Tags'),
  backdrop: Center(child: _body(context, 'The screen the sheet sits over')),
  builder: (_) => _body(context, 'Raised on the first frame.'),
  onDismiss: () {},
);
