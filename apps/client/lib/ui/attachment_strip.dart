import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../state/app_state.dart';

/// Images referenced by the open note, shown as thumbnails under the editor.
///
/// They can't be rendered *inline*. A `WidgetSpan` contributes exactly one
/// character to the span tree, while `![alt](path)` is many — and the editor's
/// buffer has to match what it renders character for character or every caret
/// offset past the image is wrong. True inline images need the block-based
/// editor (see the M0 findings).
///
/// A strip is the honest version: the markdown stays visible and editable, and
/// the pictures are actually visible.
class AttachmentStrip extends ConsumerWidget {
  const AttachmentStrip({super.key, required this.body});

  /// The note's body — parsed for `![alt](path)` links.
  final String body;

  static final _imageLink = RegExp(r'!\[[^\]]*\]\(([^)\s]+)\)');
  static const _imageExtensions = {'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'};

  static List<String> imagePaths(String body) {
    return _imageLink
        .allMatches(body)
        .map((m) => m.group(1)!)
        .where((p) => !p.startsWith('http'))
        .where((p) {
          final ext = p.rsplit('.');
          return ext != null && _imageExtensions.contains(ext);
        })
        .toSet()
        .toList();
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final api = ref.watch(apiProvider);
    final vaultId = ref.watch(activeVaultProvider);
    if (api == null) return const SizedBox.shrink();

    final paths = imagePaths(body);
    if (paths.isEmpty) return const SizedBox.shrink();

    final theme = Theme.of(context);
    return Container(
      height: 96,
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: theme.dividerColor)),
      ),
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 10),
        itemCount: paths.length,
        separatorBuilder: (_, _) => const SizedBox(width: 10),
        itemBuilder: (c, i) {
          final url = api.attachmentUrl(vaultId, paths[i]);
          return InkWell(
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => _Viewer(url: url, title: paths[i]),
              ),
            ),
            child: ClipRRect(
              borderRadius: BorderRadius.circular(6),
              child: Image.network(
                url.toString(),
                width: 76,
                height: 76,
                fit: BoxFit.cover,
                errorBuilder: (c, _, _) => Container(
                  width: 76,
                  height: 76,
                  color: theme.colorScheme.surfaceContainerHighest,
                  child: Icon(
                    Icons.broken_image_outlined,
                    size: 20,
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

class _Viewer extends StatelessWidget {
  const _Viewer({required this.url, required this.title});

  final Uri url;
  final String title;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: Text(title, overflow: TextOverflow.ellipsis)),
      backgroundColor: Colors.black,
      body: Center(
        child: InteractiveViewer(
          maxScale: 6,
          child: Image.network(
            url.toString(),
            errorBuilder: (c, _, _) => const Padding(
              padding: EdgeInsets.all(24),
              child: Text(
                'Could not load this attachment.',
                style: TextStyle(color: Colors.white70),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

extension on String {
  /// The lowercased extension, or null when there isn't one.
  String? rsplit(String sep) {
    final i = lastIndexOf(sep);
    return i < 0 || i == length - 1 ? null : substring(i + 1).toLowerCase();
  }
}
