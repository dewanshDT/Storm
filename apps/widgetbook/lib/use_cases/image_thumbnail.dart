import 'package:flutter/material.dart';
import 'package:storm/ui/widgets.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Widgets]';
const _image = AssetImage('assets/icon/storm_icon_full.png', package: 'storm');

@widgetbook.UseCase(name: 'default', type: ImageThumbnail, path: _path)
Widget imageThumbnailDefault(BuildContext context) => Padding(
  padding: const EdgeInsets.all(16),
  child: ImageThumbnail(image: _image, onRemove: () {}),
);
