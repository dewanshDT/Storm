import 'package:flutter/material.dart';
import 'package:storm/ui/panels.dart';
import 'package:storm_widgetbook/stateful_value.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Panels]';

Widget _tabs(
  List<(String, String)> tabs,
  String initial, {
  bool wrap = false,
  double? width,
}) => StatefulValue<String>(
  initial: initial,
  builder: (value, set) => Padding(
    padding: const EdgeInsets.all(16),
    child: SizedBox(
      width: width,
      child: PanelTabs<String>(
        tabs: tabs,
        selected: value,
        onSelected: set,
        wrap: wrap,
      ),
    ),
  ),
);

const _three = [
  ('props', 'Properties'),
  ('links', 'Links'),
  ('about', 'About'),
];

@widgetbook.UseCase(name: 'default', type: PanelTabs, path: _path)
Widget panelTabsDefault(BuildContext context) => _tabs(
  _three,
  context.knobs.object.dropdown<String>(
    label: 'selected',
    options: [for (final (v, _) in _three) v],
    initialOption: 'props',
  ),
  wrap: context.knobs.boolean(label: 'wrap'),
);

@widgetbook.UseCase(name: 'many tabs (wrap)', type: PanelTabs, path: _path)
Widget panelTabsWrap(BuildContext context) => _tabs(
  const [
    ('props', 'Properties'),
    ('links', 'Links'),
    ('mentions', 'Mentions'),
    ('history', 'History'),
    ('about', 'About'),
  ],
  'props',
  wrap: true,
  width: 260,
);

@widgetbook.UseCase(name: 'long labels (scroll)', type: PanelTabs, path: _path)
Widget panelTabsScroll(BuildContext context) => _tabs(
  const [
    ('a', 'A tab with a very long label'),
    ('b', 'Another long label for a narrow panel'),
  ],
  'a',
  width: 260,
);

@widgetbook.UseCase(name: 'long labels (wrap)', type: PanelTabs, path: _path)
Widget panelTabsLong(BuildContext context) => _tabs(
  const [
    ('a', 'A tab with a very long label'),
    ('b', 'Another long label for a narrow panel'),
  ],
  'a',
  wrap: true,
  width: 260,
);

@widgetbook.UseCase(name: 'default', type: MonoTag, path: _path)
Widget monoTagDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: MonoTag(context.knobs.string(label: 'text', initialValue: 'Work')),
);

@widgetbook.UseCase(name: 'long text (overflow)', type: MonoTag, path: _path)
Widget monoTagLong(BuildContext context) => const Padding(
  padding: EdgeInsets.all(16),
  child: SizedBox(width: 120, child: MonoTag('A vault with a very long name')),
);
