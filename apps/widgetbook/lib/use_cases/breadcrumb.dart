import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';

Widget _crumbs(List<Crumb> crumbs, {double? width}) => Padding(
  padding: const EdgeInsets.all(16),
  child: SizedBox(
    width: width,
    child: Breadcrumb(crumbs: crumbs),
  ),
);

@widgetbook.UseCase(name: 'default', type: Breadcrumb, path: _path)
Widget breadcrumbDefault(BuildContext context) {
  final depth = context.knobs.int.slider(
    label: 'depth',
    initialValue: 3,
    min: 1,
    max: 8,
  );
  return _crumbs([
    for (var i = 1; i <= depth; i++)
      Crumb('Folder $i', onTap: i < depth ? () {} : null),
  ]);
}

@widgetbook.UseCase(name: 'single crumb', type: Breadcrumb, path: _path)
Widget breadcrumbSingle(BuildContext context) =>
    _crumbs(const [Crumb('Vaults')]);

@widgetbook.UseCase(name: 'long trail (scrolls)', type: Breadcrumb, path: _path)
Widget breadcrumbLong(BuildContext context) => _crumbs([
  Crumb('Vaults', onTap: () {}),
  Crumb('A vault with a long name', onTap: () {}),
  Crumb('Quarterly planning', onTap: () {}),
  Crumb('Offsite notes and action items', onTap: () {}),
  const Crumb('Decisions nobody has picked up yet'),
], width: 280);
