import 'package:flutter/material.dart';
import 'package:storm/ui/panels.dart';
import 'package:widgetbook/widgetbook.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Panels]';

Widget _list(List<(String, String)> rows) => Padding(
  padding: const EdgeInsets.all(16),
  child: KeyValueList(rows: rows),
);

@widgetbook.UseCase(name: 'default', type: KeyValueList, path: _path)
Widget keyValueListDefault(BuildContext context) => _list([
  (
    context.knobs.string(label: 'key', initialValue: 'Host'),
    context.knobs.string(label: 'value', initialValue: 'homelab-01'),
  ),
  ('Runtime', '0.19.1'),
  ('Last seen', '2 minutes ago'),
]);

@widgetbook.UseCase(name: 'empty', type: KeyValueList, path: _path)
Widget keyValueListEmpty(BuildContext context) => _list(const []);

@widgetbook.UseCase(
  name: 'long text (overflow)',
  type: KeyValueList,
  path: _path,
)
Widget keyValueListLong(BuildContext context) => _list(const [
  (
    'Working directory',
    '/home/someone/storm/workspaces/a-project-with-a-very-long-name/'
        'and/a/deeply/nested/folder/that/keeps/going',
  ),
  (
    'A key that is itself much longer than any panel would expect to hold',
    'Short',
  ),
  (
    'Unbroken',
    'a_value_with_no_spaces_at_all_so_it_cannot_wrap_anywhere_'
        'and_must_be_handled_by_the_layout',
  ),
]);
