import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:storm/ui/shell/sidebar_rows.dart';
import 'package:storm/ui/tokens.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Sidebar]';

Widget _pad(Widget child, {double? width}) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(width: width, child: child),
);

@widgetbook.UseCase(name: 'default', type: VaultTag, path: _path)
Widget vaultTagDefault(BuildContext context) => _pad(
  VaultTag(
    context.knobs.string(label: 'name', initialValue: 'Work'),
    color: context.knobs.boolean(label: 'accent colour')
        ? context.tokens.accent
        : null,
  ),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: VaultTag, path: _path)
Widget vaultTagLong(BuildContext context) =>
    _pad(const VaultTag('A vault with a very long name'), width: 120);

@widgetbook.UseCase(name: 'default', type: Twisty, path: _path)
Widget twistyDefault(BuildContext context) => _pad(
  Twisty(open: context.knobs.boolean(label: 'open', initialValue: true)),
);

@widgetbook.UseCase(name: 'open and closed', type: Twisty, path: _path)
Widget twistyBoth(BuildContext context) => _pad(
  const Row(
    mainAxisSize: MainAxisSize.min,
    children: [Twisty(open: true), SizedBox(width: 12), Twisty(open: false)],
  ),
);

@widgetbook.UseCase(name: 'default', type: UnseenDot, path: _path)
Widget unseenDotDefault(BuildContext context) => _pad(
  UnseenDot(
    message: context.knobs.object.dropdown<String>(
      label: 'message',
      options: const [UnseenDot.noteMessage, UnseenDot.sessionMessage],
      initialOption: UnseenDot.noteMessage,
    ),
  ),
);

Widget _row(
  BuildContext context, {
  String label = 'Projects',
  bool selected = false,
  bool unseen = false,
  bool leading = true,
  bool trailing = false,
  bool tappable = true,
  double indent = 0,
  List<double> guides = const [],
}) {
  final t = context.tokens;
  return _pad(
    SidebarRow(
      label: label,
      leading: leading
          ? Icon(LucideIcons.folder, size: 16, color: t.text2)
          : null,
      trailing: trailing ? const Twisty(open: false) : null,
      selected: selected,
      unseen: unseen,
      indent: indent,
      guides: guides,
      onTap: tappable ? () {} : null,
    ),
    width: 260,
  );
}

@widgetbook.UseCase(name: 'default', type: SidebarRow, path: _path)
Widget sidebarRowDefault(BuildContext context) => _row(
  context,
  label: context.knobs.string(label: 'label', initialValue: 'Projects'),
  selected: context.knobs.boolean(label: 'selected'),
  unseen: context.knobs.boolean(label: 'unseen'),
  leading: context.knobs.boolean(label: 'leading', initialValue: true),
  trailing: context.knobs.boolean(label: 'trailing'),
  tappable: context.knobs.boolean(label: 'tappable', initialValue: true),
  indent: context.knobs.double.slider(
    label: 'indent',
    initialValue: 0,
    min: 0,
    max: 64,
  ),
);

@widgetbook.UseCase(name: 'selected', type: SidebarRow, path: _path)
Widget sidebarRowSelected(BuildContext context) =>
    _row(context, selected: true);

@widgetbook.UseCase(name: 'unseen change', type: SidebarRow, path: _path)
Widget sidebarRowUnseen(BuildContext context) =>
    _row(context, unseen: true, trailing: true);

@widgetbook.UseCase(name: 'nested with guides', type: SidebarRow, path: _path)
Widget sidebarRowNested(BuildContext context) =>
    _row(context, label: 'Decisions', indent: 32, guides: const [14, 30]);

@widgetbook.UseCase(name: 'disabled (no onTap)', type: SidebarRow, path: _path)
Widget sidebarRowDisabled(BuildContext context) =>
    _row(context, tappable: false);

@widgetbook.UseCase(name: 'long text (overflow)', type: SidebarRow, path: _path)
Widget sidebarRowLong(BuildContext context) => _row(
  context,
  label: 'A folder with a name far too long to fit in the sidebar',
  unseen: true,
  trailing: true,
);
