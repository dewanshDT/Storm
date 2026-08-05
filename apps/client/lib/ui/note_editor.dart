import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../editor/markdown_theme.dart';
import '../editor/storm_markdown_controller.dart';
import '../state/app_state.dart';
import '../state/note_session.dart';
import 'editor_toolbar.dart';
import 'note_properties.dart';
import 'theme.dart';

/// The editing pane.
///
/// Bridges two things that both want to own the text: Flutter's
/// [TextEditingController] and the [NoteSession] that talks to the server.
/// The session wins — when it raises its revision counter, the server has
/// reconciled our text against a version we never saw, and the buffer must be
/// replaced rather than kept.
class NoteEditor extends ConsumerStatefulWidget {
  const NoteEditor({super.key, this.onFollowLink, this.showToolbar = false});

  /// Whether to show the formatting toolbar.
  ///
  /// Decided by the screen rather than here, because the answer depends on the
  /// keyboard inset and this widget lives inside a Scaffold body where that is
  /// no longer readable. See `keyboardIsOpen`.
  final bool showToolbar;

  /// Called with a `[[target]]` the user asked to follow.
  ///
  /// The editor finds the link; it does not decide what opening one means.
  /// That belongs to the screen, which owns navigation.
  final void Function(String target)? onFollowLink;

  @override
  ConsumerState<NoteEditor> createState() => _NoteEditorState();
}

class _NoteEditorState extends ConsumerState<NoteEditor> {
  late StormMarkdownController _controller;
  final _focus = FocusNode();
  int _seenRevision = -1;

  /// When true the editor shows the whole file, frontmatter included, so the
  /// raw YAML can be corrected. The properties panel can't write values back —
  /// doing so would mean re-serialising the user's YAML, which reorders keys
  /// and drops comments.
  bool _rawMode = false;

  /// Whether this note is past the size where styling is dropped.
  ///
  /// Derived from the line count rather than read back off the controller.
  /// `lastDegraded` is written *during* `buildTextSpan`, so reading it here
  /// would be a frame behind and reacting to it would risk a rebuild loop —
  /// the exact shape of bug that once wiped every note in the vault.
  bool _isDegraded(NoteSession session) =>
      '\n'.allMatches(_textFor(session)).length + 1 >
      StormMarkdownController.maxStyledLines;

  /// What the editor should currently contain for [session].
  String _textFor(NoteSession session) =>
      _rawMode ? session.buffer : session.body;

  @override
  void initState() {
    super.initState();
    _controller = StormMarkdownController(
      theme: MarkdownTheme.light(const TextStyle(fontSize: 16)),
    );
    _controller.addListener(_onLocalEdit);
  }

  @override
  void dispose() {
    _controller.removeListener(_onLocalEdit);
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  /// The last text this widget knows about, so a controller notification that
  /// isn't a text change can't be mistaken for typing.
  ///
  /// [TextEditingController] notifies on selection changes and on anything
  /// else that touches its value. Treating every notification as an edit once
  /// reported the still-empty controller as a deletion of the whole note, and
  /// the resulting empty save looped through the server and back.
  String _lastText = '';

  void _onLocalEdit() {
    if (_controller.text == _lastText) return;
    _lastText = _controller.text;

    final session = ref.read(noteSessionProvider);
    if (!session.isOpen) return;
    // The controller holds the body; the session re-attaches the frontmatter.
    if (_rawMode) {
      session.edit(_controller.text);
    } else {
      session.editBody(_controller.text);
    }
  }

  /// Follows a wikilink the caret has just landed inside.
  ///
  /// A tap inside an editable field is consumed for caret placement, so a
  /// gesture recogniser on the span never fires. Letting the tap do its normal
  /// job and then asking where the caret ended up gets the same result without
  /// fighting the selection gesture.
  ///
  /// On touch a tap follows the link. With a pointer it needs Cmd/Ctrl, because
  /// clicking into text to edit it is the overwhelmingly common intent and
  /// stealing that would make the note unclickable.
  void _onEditorTap() {
    final follow = widget.onFollowLink;
    if (follow == null) return;

    final hit = wikilinkAt(_controller.text, _controller.selection.baseOffset);
    if (hit == null) return;

    final isTouch =
        defaultTargetPlatform == TargetPlatform.android ||
        defaultTargetPlatform == TargetPlatform.iOS;
    final modified = HardwareKeyboard.instance.isMetaPressed ||
        HardwareKeyboard.instance.isControlPressed;
    if (!isTouch && !modified) return;

    follow(hit.target);
  }

  /// Adds "Open note" to the selection toolbar when the caret is in a link.
  ///
  /// The sanctioned extension point: unlike a gesture recogniser it does not
  /// compete with selection, and it gives pointer users a way through that
  /// doesn't depend on knowing about a modifier key.
  Widget _contextMenu(BuildContext context, EditableTextState state) {
    final items = [...state.contextMenuButtonItems];
    final follow = widget.onFollowLink;
    final hit = follow == null
        ? null
        : wikilinkAt(_controller.text, _controller.selection.baseOffset);

    if (hit != null) {
      items.insert(
        0,
        ContextMenuButtonItem(
          label: 'Open ${hit.target}',
          onPressed: () {
            state.hideToolbar();
            follow!(hit.target);
          },
        ),
      );
    }
    return AdaptiveTextSelectionToolbar.buttonItems(
      anchors: state.contextMenuAnchors,
      buttonItems: items,
    );
  }

  /// Pulls server text into the editor, keeping the caret where it can.
  void _adoptServerText(String text) {
    final previousOffset = _controller.selection.baseOffset;
    _controller.removeListener(_onLocalEdit);
    _lastText = text;
    _controller.value = TextEditingValue(
      text: text,
      // Clamp rather than reset to 0: after a clean merge the user's caret is
      // usually still meaningful, and dumping them at the top of a long note
      // would be worse than being a few characters off.
      selection: TextSelection.collapsed(
        offset: previousOffset.clamp(0, text.length),
      ),
    );
    _controller.addListener(_onLocalEdit);
  }

  @override
  Widget build(BuildContext context) {
    final session = ref.watch(noteSessionProvider);
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final dark = Theme.of(context).brightness == Brightness.dark;

    // Serif for the note body only — it is the one place in the app that is
    // prose rather than chrome, and the distinction is the point.
    final base = TextStyle(
      fontFamily: StormTheme.bodyFamily,
      fontSize: settings.fontSize + 1,
      height: 1.6,
      color: Theme.of(context).colorScheme.onSurface,
    );
    _controller.theme = dark
        ? MarkdownTheme.dark(base)
        : MarkdownTheme.light(base);

    // The session replaced the buffer (opened a note, or adopted a merge).
    if (session.revision != _seenRevision) {
      _seenRevision = session.revision;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        final want = _textFor(session);
        if (mounted && _controller.text != want) {
          _adoptServerText(want);
        }
      });
    }

    if (!session.isOpen) {
      return _Placeholder(error: session.error);
    }

    return Column(
      children: [
        _StatusBar(session: session),
        if (!_rawMode)
          NoteProperties(
            frontmatter: session.frontmatter,
            onEditRaw: () {
              setState(() => _rawMode = true);
              _adoptServerText(session.buffer);
            },
          ),
        if (_rawMode)
          _RawBanner(
            onDone: () {
              setState(() => _rawMode = false);
              _adoptServerText(session.body);
            },
          ),
        if (_isDegraded(session)) const _DegradedNotice(),
        if (session.notice != null)
          _Notice(
            message: session.notice!,
            isConflict: session.hasConflict,
            onDismiss: session.dismissNotice,
          ),
        Expanded(
          child: Scrollbar(
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(28, 18, 28, 120),
              child: Center(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 820),
                  child: TextField(
                    controller: _controller,
                    focusNode: _focus,
                    maxLines: null,
                    cursorWidth: 2,
                    keyboardType: TextInputType.multiline,
                    textCapitalization: TextCapitalization.sentences,
                    onTap: _onEditorTap,
                    contextMenuBuilder: _contextMenu,
                    decoration: const InputDecoration(
                      border: InputBorder.none,
                      isDense: true,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
        // Trades places with the nav bubble, which hides on the same signal.
        if (widget.showToolbar) EditorToolbar(controller: _controller),
      ],
    );
  }
}

/// Shown when a note is too long to style.
///
/// Without this the styling simply vanishes on a large note, which reads as a
/// bug rather than as the deliberate trade it is.
class _DegradedNotice extends StatelessWidget {
  const _DegradedNotice();

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      width: double.infinity,
      color: scheme.tertiaryContainer,
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 9),
      child: Row(
        children: [
          Icon(Icons.speed, size: 16, color: scheme.onTertiaryContainer),
          const SizedBox(width: 10),
          Expanded(
            child: Text(
              'Formatting is off above '
              '${StormMarkdownController.maxStyledLines} lines, to keep typing '
              'responsive. The note itself is unchanged.',
              style: TextStyle(
                color: scheme.onTertiaryContainer,
                fontSize: 12.5,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Shown while the frontmatter is being edited as text.
class _RawBanner extends StatelessWidget {
  const _RawBanner({required this.onDone});

  final VoidCallback onDone;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      width: double.infinity,
      color: scheme.secondaryContainer,
      padding: const EdgeInsets.fromLTRB(20, 8, 8, 8),
      child: Row(
        children: [
          Icon(Icons.data_object, size: 16, color: scheme.onSecondaryContainer),
          const SizedBox(width: 10),
          Expanded(
            child: Text(
              'Editing raw frontmatter',
              style: TextStyle(
                color: scheme.onSecondaryContainer,
                fontSize: 13,
              ),
            ),
          ),
          TextButton(onPressed: onDone, child: const Text('Done')),
        ],
      ),
    );
  }
}

class _StatusBar extends StatelessWidget {
  const _StatusBar({required this.session});

  final NoteSession session;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final (label, color) = switch (session.saveState) {
      SaveState.idle => ('', theme.colorScheme.onSurfaceVariant),
      SaveState.dirty => ('Unsaved', theme.colorScheme.onSurfaceVariant),
      SaveState.saving => ('Saving…', theme.colorScheme.onSurfaceVariant),
      SaveState.saved => ('Saved', theme.colorScheme.primary),
      SaveState.queued => ('Queued — offline', theme.colorScheme.tertiary),
      SaveState.failed => ('Failed', theme.colorScheme.error),
    };

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 8),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: theme.dividerColor)),
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(
              session.meta?.path ?? '',
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          if (session.error != null) ...[
            Icon(Icons.error_outline, size: 14, color: theme.colorScheme.error),
            const SizedBox(width: 5),
            Text(
              session.error!,
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.error,
              ),
            ),
            const SizedBox(width: 12),
          ],
          Text(
            'v${session.baseVersion}',
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(width: 12),
          Text(label, style: theme.textTheme.bodySmall?.copyWith(color: color)),
        ],
      ),
    );
  }
}

class _Notice extends StatelessWidget {
  const _Notice({
    required this.message,
    required this.isConflict,
    required this.onDismiss,
  });

  final String message;
  final bool isConflict;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final bg = isConflict ? scheme.errorContainer : scheme.secondaryContainer;
    final fg = isConflict
        ? scheme.onErrorContainer
        : scheme.onSecondaryContainer;

    return Container(
      width: double.infinity,
      color: bg,
      padding: const EdgeInsets.fromLTRB(20, 10, 8, 10),
      child: Row(
        children: [
          Icon(
            isConflict ? Icons.merge_type : Icons.info_outline,
            size: 16,
            color: fg,
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Text(message, style: TextStyle(color: fg, fontSize: 13)),
          ),
          IconButton(
            icon: Icon(Icons.close, size: 16, color: fg),
            onPressed: onDismiss,
            visualDensity: VisualDensity.compact,
          ),
        ],
      ),
    );
  }
}

class _Placeholder extends StatelessWidget {
  const _Placeholder({this.error});

  final String? error;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              error != null ? Icons.error_outline : Icons.description_outlined,
              size: 36,
              color: error != null
                  ? theme.colorScheme.error
                  : theme.colorScheme.onSurfaceVariant,
            ),
            const SizedBox(height: 12),
            Text(
              error ?? 'Select a note to start editing',
              textAlign: TextAlign.center,
              style: theme.textTheme.bodyMedium?.copyWith(
                color: error != null
                    ? theme.colorScheme.error
                    : theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
