import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../editor/list_continuation.dart';
import '../editor/markdown_theme.dart';
import '../editor/storm_markdown_controller.dart';
import '../state/app_state.dart';
import '../state/note_session.dart';
import 'breakpoints.dart';
import 'editor_toolbar.dart';
import 'widgets.dart';
import 'states.dart';
import 'tokens.dart';
import 'wikilink_suggestions.dart';

/// The editing pane.
///
/// Bridges two things that both want to own the text: Flutter's
/// [TextEditingController] and the [NoteSession] that talks to the server.
/// The session wins — when it raises its revision counter, the server has
/// reconciled our text against a version we never saw, and the buffer must be
/// replaced rather than kept.
class NoteEditor extends ConsumerStatefulWidget {
  const NoteEditor({
    super.key,
    this.onFollowLink,
    this.showToolbar = false,
    this.footer,
    this.onActions,
  });

  /// Pin, attach, rename, delete — reached by long-pressing the status line.
  ///
  /// The status line is the note's own chrome at *both* widths; the phone's
  /// header row does not exist at desk width, and the actions have to.
  final VoidCallback? onActions;

  /// Attachments and mentions, which the design puts *inside* the scroll
  /// after the prose rather than pinned under it.
  final Widget? footer;

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

  // There is no raw-YAML mode. Frontmatter is edited only through the
  // properties list, which shows every key in the block — including the ones
  // it will not write — so nothing is reachable only by editing text.

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
  ///
  /// Always the body. The frontmatter is not in the buffer at all — the
  /// controller's text has to match what it renders character for character,
  /// so anything hidden must be absent rather than merely unstyled.
  String _textFor(NoteSession session) => session.body;

  @override
  void initState() {
    super.initState();
    _controller = StormMarkdownController(
      // Replaced on the first build with the themed one. A size here would
      // be a number the token layer cannot reach, and it is never painted.
      theme: MarkdownTheme.light(const TextStyle()),
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
    session.editBody(_controller.text);
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
    final modified =
        HardwareKeyboard.instance.isMetaPressed ||
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

    // The note body only — it is the one place in the app that is prose
    // rather than chrome, and the distinction is the point. Which face is the
    // user's choice; the default is the bundled serif.
    final base = TextStyle(
      fontFamily: settings.bodyFont.family,
      fontSize: settings.fontSize,
      height: 1.65,
      color: context.tokens.text,
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
      return session.error == null
          ? const EmptyState(
              icon: Icons.article_outlined,
              title: 'Select a note to start editing',
            )
          : EmptyState(
              icon: Icons.error_outline,
              title: 'This note would not open',
              detail: session.error,
            );
    }

    final t = context.tokens;
    // 40 at desk width, as the prototype has it; on a phone the column is the
    // screen and 40 a side would leave nothing for the words.
    final inset = context.isExpanded ? kEditorInset : t.sp * 2.5;
    // The pane's own top padding, all of it — the shell contributes nothing
    // at this width. Getting it wrong is visible from across the room: the
    // version line and the sidebar's vault name are meant to share a
    // baseline, which is what 28 here and 18-around-a-34px-badge there both
    // land on.
    final topInset = context.isExpanded ? kEditorTopInset : 0.0;

    return Column(
      children: [
        GestureDetector(
          key: const Key('note-actions'),
          onLongPress: widget.onActions,
          behavior: HitTestBehavior.opaque,
          child: Padding(
            // The same inset as the prose below it: the design lines the
            // version up with the note's first character.
            padding: EdgeInsets.fromLTRB(inset, topInset, inset, t.sp * 0.5),
            child: _statusBar(session),
          ),
        ),
        if (_isDegraded(session)) const _DegradedNotice(),
        if (session.hasConflict)
          ConflictCard(onDismiss: session.dismissNotice)
        else if (session.notice != null)
          _Notice(
            message: session.notice!,
            isConflict: false,
            onDismiss: session.dismissNotice,
          ),
        if (session.saveState == SaveState.queued)
          OfflineNotice(queued: ref.watch(syncEngineProvider).pendingCount),
        Expanded(
          child: Scrollbar(
            child: SingleChildScrollView(
              padding: EdgeInsets.fromLTRB(inset, t.sp, inset, t.sp * 15),
              // Left, not centred. The prototype's note pane is 604px wide, so
              // its `margin: 0 auto` inside a 640 measure never actually
              // centres anything — the prose sits 40px from the column's left
              // edge. Centring reproduces that at 1200 and nowhere else: at
              // 1700 it strands the text in the middle of an empty field.
              child: Align(
                alignment: Alignment.topLeft,
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: kEditorMeasure),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      // Properties are no longer in this column. They are a
                      // surface of their own now — a sheet on a phone, a drawer
                      // on a wide screen — because a note with a dozen keys
                      // pushed the writing off the first screen, and the design
                      // gives them their own place with a header and a close.
                      TextField(
                        // Named so tests can tell the prose apart from the
                        // property inputs above it.
                        key: const Key('note-body'),
                        controller: _controller,
                        focusNode: _focus,
                        maxLines: null,
                        cursorWidth: 2,
                        keyboardType: TextInputType.multiline,
                        textCapitalization: TextCapitalization.sentences,
                        onTap: _onEditorTap,
                        // Enter inside a list carries the list on.
                        inputFormatters: const [ListContinuationFormatter()],
                        contextMenuBuilder: _contextMenu,
                        // Every border, not just `border`. The theme sets
                        // enabledBorder and focusedBorder separately, and
                        // clearing only `border` left a rounded outline drawn
                        // around the whole note. `filled: false` for the same
                        // reason: the theme fills every field with surface2,
                        // which put the prose on a card of its own.
                        decoration: const InputDecoration(
                          border: InputBorder.none,
                          enabledBorder: InputBorder.none,
                          focusedBorder: InputBorder.none,
                          disabledBorder: InputBorder.none,
                          errorBorder: InputBorder.none,
                          focusedErrorBorder: InputBorder.none,
                          filled: false,
                          contentPadding: EdgeInsets.zero,
                          isDense: true,
                        ),
                      ),
                      if (widget.footer != null) widget.footer!,
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
        // Both belong to the keyboard: suggestions sit directly above the
        // formatting row, in the space the nav bubble gives up.
        if (widget.showToolbar) ...[
          WikilinkSuggestions(controller: _controller),
          EditorToolbar(controller: _controller, onDone: _focus.unfocus),
        ],
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
    final t = context.tokens;
    // Not amber: amber means tags and highlight. This is a note about how the
    // editor is behaving, which is neither.
    return Container(
      width: double.infinity,
      color: t.surface2,
      padding: EdgeInsets.symmetric(horizontal: t.sp * 2.5, vertical: t.sp),
      child: Row(
        children: [
          Icon(Icons.speed, size: t.bodySize, color: t.text3),
          SizedBox(width: t.sp * 1.25),
          Expanded(
            child: Text(
              'Formatting is off above '
              '${StormMarkdownController.maxStyledLines} lines, to keep typing '
              'responsive. The note itself is unchanged.',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                color: t.text2,
                fontSize: t.labelSize,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

extension on _NoteEditorState {
  /// The tone is the meaning, and the meanings are fixed: green is the server
  /// has it, amber is waiting, danger is it did not go, grey is in flight.
  Widget _statusBar(NoteSession session) {
    final (label, tone) = switch (session.saveState) {
      SaveState.idle => ('', SaveTone.working),
      SaveState.dirty => ('Unsaved', SaveTone.working),
      SaveState.saving => ('Saving…', SaveTone.working),
      SaveState.saved => ('Saved', SaveTone.good),
      SaveState.queued => ('Queued — offline', SaveTone.waiting),
      SaveState.failed => ('Failed', SaveTone.bad),
    };
    return StatusBar(
      version: session.baseVersion,
      label: label,
      tone: tone,
      error: session.error,
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
    final t = context.tokens;
    final fg = isConflict ? t.danger : t.text2;

    return Container(
      width: double.infinity,
      color: t.surface2,
      padding: EdgeInsets.fromLTRB(t.sp * 2.5, t.sp * 1.25, t.sp, t.sp * 1.25),
      child: Row(
        children: [
          Icon(
            isConflict ? Icons.merge_type : Icons.info_outline,
            size: t.bodySize,
            color: fg,
          ),
          SizedBox(width: t.sp * 1.25),
          Expanded(
            child: Text(
              message,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                color: fg,
                fontSize: t.codeSize,
              ),
            ),
          ),
          IconButton(
            icon: Icon(Icons.close, size: t.bodySize, color: fg),
            onPressed: onDismiss,
            visualDensity: VisualDensity.compact,
          ),
        ],
      ),
    );
  }
}
