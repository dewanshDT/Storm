import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:markdown/markdown.dart' as md;
import 'package:url_launcher/url_launcher.dart';

import '../../state/app_state.dart';
import '../states.dart';
import '../tokens.dart';
import '../widgets.dart';
import 'storm_markdown_style.dart';

/// Scheme used for `[[wikilink]]` targets rendered as ordinary Markdown links.
const kStormWikilinkScheme = 'storm-wikilink';

/// Read-only Storm document view over a Markdown string.
///
/// Markdown stays the source of truth — this widget only renders. Editing still
/// goes through the existing editor. Keeps image and link resolution on the
/// same attachment / wikilink paths the rest of the client already uses.
class StormMarkdownView extends ConsumerWidget {
  const StormMarkdownView({
    super.key,
    required this.markdown,
    this.onFollowLink,
    this.onOpenEdit,
  });

  /// Note body only (no frontmatter). Same string the editor edits.
  final String markdown;

  /// `[[target]]` the reader tapped.
  final void Function(String target)? onFollowLink;

  /// Escape hatch when rendering fails — switch to Edit Mode.
  final VoidCallback? onOpenEdit;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (markdown.trim().isEmpty) {
      return EmptyState(
        key: const Key('read-empty'),
        icon: LucideIcons.book_open,
        title: 'Nothing to read yet.',
        detail: 'Switch to Edit to start writing.',
        action: onOpenEdit == null ? null : 'Edit',
        onAction: onOpenEdit,
      );
    }

    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final api = ref.watch(apiProvider);
    final vaultId = ref.watch(activeVaultProvider);

    final style = stormMarkdownStyleSheet(
      context: context,
      bodyFamily: settings.bodyFont.family,
      fontSize: settings.fontSize,
    );

    try {
      return MarkdownBody(
        key: const Key('storm-markdown-body'),
        data: _withCompletedTaskStrike(markdown),
        selectable: true,
        styleSheet: style,
        styleSheetTheme: MarkdownStyleSheetBaseTheme.material,
        // Baseline alignment fights a non-text checkbox widget; start + a hair
        // of top pad optically centres the 18px box on the first text line,
        // matching `align-items: center` in the prototype.
        listItemCrossAxisAlignment: MarkdownListItemCrossAxisAlignment.start,
        extensionSet: md.ExtensionSet(
          md.ExtensionSet.gitHubFlavored.blockSyntaxes,
          <md.InlineSyntax>[
            WikilinkSyntax(),
            ...md.ExtensionSet.gitHubFlavored.inlineSyntaxes,
          ],
        ),
        checkboxBuilder: (checked) => Padding(
          // ~((16 * 1.65) - 18) / 2 ≈ 4 — keeps the box on the text band.
          padding: const EdgeInsets.only(top: 4),
          child: StormCheckbox(
            value: checked,
            // Read-only for this phase — no second mutation path.
            onChanged: null,
            size: kStormMarkdownCheckboxSize,
          ),
        ),
        imageBuilder: (uri, title, alt) => _MarkdownImage(
          uri: uri,
          alt: alt,
          attachmentUrl: api == null || vaultId.isEmpty
              ? null
              : (path) => api.attachmentUrl(vaultId, path),
        ),
        onTapLink: (text, href, title) =>
            _onTapLink(context, text: text, href: href),
      );
    } catch (e) {
      return EmptyState(
        key: const Key('read-error'),
        icon: LucideIcons.circle_alert,
        title: 'Unable to render this note.',
        detail: '$e',
        action: onOpenEdit == null ? null : 'Open in Edit Mode',
        onAction: onOpenEdit,
      );
    }
  }

  void _onTapLink(
    BuildContext context, {
    required String text,
    required String? href,
  }) {
    if (href == null || href.isEmpty) return;

    const prefix = '$kStormWikilinkScheme:';
    if (href.startsWith(prefix)) {
      final target = Uri.decodeComponent(href.substring(prefix.length));
      if (target.isNotEmpty) onFollowLink?.call(target);
      return;
    }

    final uri = Uri.tryParse(href);
    if (uri != null && (uri.scheme == 'http' || uri.scheme == 'https')) {
      // Fire-and-forget: a dead link is not worth blocking the reader.
      launchUrl(uri, mode: LaunchMode.externalApplication);
    }
  }
}

/// Prototype completed tasks are struck through at reduced opacity.
///
/// GFM gives us a checkbox node but leaves the label plain; wrap checked
/// item text in `~~…~~` for display only so Read Mode matches the design
/// without writing anything back to the note.
final _checkedTask = RegExp(
  r'^(\s*[-*+]\s+)\[x\]\s+(?!~~)(.+?)\s*$',
  multiLine: true,
  caseSensitive: false,
);

String _withCompletedTaskStrike(String markdown) =>
    markdown.replaceAllMapped(_checkedTask, (m) => '${m[1]}[x] ~~${m[2]}~~');

/// Turns `[[Target]]` into an ordinary Markdown link with a Storm scheme.
///
/// Placed ahead of the GFM inline set so `[[…]]` is not eaten as a broken
/// reference link.
class WikilinkSyntax extends md.InlineSyntax {
  WikilinkSyntax() : super(r'\[\[([^\[\]\n]+)\]\]');

  @override
  bool onMatch(md.InlineParser parser, Match match) {
    final target = match[1]!.trim();
    if (target.isEmpty) return false;
    final el = md.Element.text('a', target);
    el.attributes['href'] =
        '$kStormWikilinkScheme:${Uri.encodeComponent(target)}';
    parser.addNode(el);
    return true;
  }
}

class _MarkdownImage extends StatelessWidget {
  const _MarkdownImage({
    required this.uri,
    required this.alt,
    required this.attachmentUrl,
  });

  final Uri uri;
  final String? alt;
  final Uri Function(String path)? attachmentUrl;

  static const _imageExtensions = {'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'};

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final url = _resolveUrl();
    if (url == null) {
      return _ImageFallback(
        message: alt?.isNotEmpty == true ? alt! : 'Missing image',
      );
    }

    return Padding(
      padding: EdgeInsets.symmetric(vertical: t.sp),
      child: GestureDetector(
        onTap: () => Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => _ImageViewer(url: url, title: alt ?? uri.path),
          ),
        ),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(t.rControl),
          child: Image.network(
            url.toString(),
            fit: BoxFit.contain,
            loadingBuilder: (context, child, progress) {
              if (progress == null) return child;
              return Container(
                height: t.sp * 16,
                alignment: Alignment.center,
                color: t.surface2,
                child: SizedBox(
                  width: t.sp * 2.5,
                  height: t.sp * 2.5,
                  child: CircularProgressIndicator(
                    strokeWidth: t.bw * 2,
                    color: t.accent,
                  ),
                ),
              );
            },
            errorBuilder: (context, error, stack) => _ImageFallback(
              message: alt?.isNotEmpty == true ? alt! : 'Could not load image',
            ),
          ),
        ),
      ),
    );
  }

  Uri? _resolveUrl() {
    if (uri.scheme == 'http' || uri.scheme == 'https') return uri;

    // Vault-relative attachment path, the form the editor inserts on upload.
    final path = uri.hasScheme ? uri.toString() : uri.path;
    if (path.isEmpty) return null;
    final ext = path.contains('.') ? path.split('.').last.toLowerCase() : '';
    if (ext.isNotEmpty && !_imageExtensions.contains(ext)) return null;
    return attachmentUrl?.call(path);
  }
}

class _ImageFallback extends StatelessWidget {
  const _ImageFallback({required this.message});

  final String message;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      width: double.infinity,
      padding: EdgeInsets.all(t.sp * 2),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Row(
        children: [
          Icon(LucideIcons.image_off, size: t.bodySize, color: t.text3),
          SizedBox(width: t.sp),
          Expanded(
            child: Text(
              message,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.text3,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _ImageViewer extends StatelessWidget {
  const _ImageViewer({required this.url, required this.title});

  final Uri url;
  final String title;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Scaffold(
      appBar: AppBar(title: Text(title, overflow: TextOverflow.ellipsis)),
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
