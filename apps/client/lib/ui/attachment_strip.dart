import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../state/app_state.dart';
import 'breakpoints.dart';
import 'tokens.dart';

/// Images referenced by the open note, shown as thumbnails under the editor.
///
/// They can't be rendered *inline*. A `WidgetSpan` contributes exactly one
/// character to the span tree, while `![alt](path)` is many — and the editor's
/// buffer has to match what it renders character for character or every caret
/// offset past the image is wrong. True inline images need the block-based
/// editor (see the M0 findings).
///
/// A strip is the honest version: the markdown stays visible and editable, and
/// the pictures are actually visible. It sits inside the note's scroll, after
/// the prose, rather than pinned under it.
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

    final t = context.tokens;
    final side = t.sp * (context.isExpanded ? 11 : 9.5);

    return Padding(
      padding: EdgeInsets.only(top: t.sp * 2.5),
      child: Wrap(
        spacing: t.sp * 1.25,
        runSpacing: t.sp * 1.25,
        children: [
          for (final path in paths)
            Builder(
              builder: (c) {
                final url = api.attachmentUrl(vaultId, path);
                return InkWell(
                  onTap: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => _Viewer(url: url, title: path),
                    ),
                  ),
                  borderRadius: BorderRadius.circular(t.rControl),
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(t.rControl),
                    child: Image.network(
                      url.toString(),
                      width: side,
                      height: side,
                      fit: BoxFit.cover,
                      errorBuilder: (c, _, _) => Container(
                        width: side,
                        height: side,
                        alignment: Alignment.center,
                        color: t.surface2,
                        child: Icon(
                          Icons.broken_image_outlined,
                          size: t.bodySize,
                          color: t.text3,
                        ),
                      ),
                    ),
                  ),
                );
              },
            ),
        ],
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
    final t = context.tokens;
    return Scaffold(
      appBar: AppBar(title: Text(title, overflow: TextOverflow.ellipsis)),
      // A viewer is the one surface that should not be themed: a picture is
      // judged against a neutral ground, not against the app's.
      backgroundColor: const Color(0xFF000000),
      body: Center(
        child: InteractiveViewer(
          maxScale: 6,
          child: Image.network(
            url.toString(),
            errorBuilder: (c, _, _) => Padding(
              padding: EdgeInsets.all(t.cardPad),
              child: Text(
                'Could not load this attachment.',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: const Color(0xB3FFFFFF),
                ),
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
