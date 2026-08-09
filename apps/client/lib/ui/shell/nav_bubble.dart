import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../breakpoints.dart';
import '../icons.dart';
import '../tokens.dart';
import 'vault_actions.dart';

/// Whether a soft keyboard is currently covering the bottom of the screen.
///
/// **Call this above a `Scaffold`, not from inside its body.** Both of the
/// obvious ways to ask are wrong from inside one:
///
///  * `MediaQuery.viewInsetsOf` reads zero, because `resizeToAvoidBottomInset`
///    works by *removing* the bottom inset from the body's MediaQuery — that
///    removal is the resize.
///  * `View.of(context).viewInsets` reads the right number but never rebuilds:
///    view metrics are not an inherited dependency, so the widget would keep
///    whatever answer it got the first time.
///
/// Above the Scaffold the MediaQuery is intact *and* depending on it rebuilds,
/// so the screen decides and passes the answer down. The nav bubble and the
/// formatting toolbar both hang off this one call per screen, which is what
/// keeps them from ever being on screen together.
bool keyboardIsOpen(BuildContext context) =>
    MediaQuery.viewInsetsOf(context).bottom > 0;

/// The floating navigation bubble.
///
/// Directory, Search, New note, New folder and Context, always shown. It used
/// to collapse to a single `…` until tapped, which cost a tap before every
/// navigation and hid where you could go — the bar is small enough that
/// hiding it bought nothing.
///
/// A phone shape. On a wide screen the same actions live in the sidebar's
/// toolbar instead, so this hides itself rather than floating in the middle of
/// a 2000px window. Both draw [vaultActions], so the two placements cannot
/// drift into offering different things.
class NavBubble extends ConsumerStatefulWidget {
  const NavBubble({super.key});

  @override
  ConsumerState<NavBubble> createState() => _NavBubbleState();
}

class _NavBubbleState extends ConsumerState<NavBubble> {
  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final uri = GoRouterState.of(context).uri;

    // The sidebar carries these on a wide screen.
    if (context.isExpanded) return const SizedBox.shrink();

    return SafeArea(
      minimum: EdgeInsets.only(bottom: t.sp * 2),
      child: Align(
        alignment: Alignment.bottomCenter,
        child: AnimatedSize(
          duration: t.duration,
          curve: Curves.easeOutCubic,
          child: Container(
            decoration: BoxDecoration(
              color: t.surface2,
              // A pill, where the corner bubbles are rounded squares. The
              // shape difference is deliberate grammar: corners are places,
              // the pill is an action bar.
              borderRadius: BorderRadius.circular(999),
              border: Border.all(color: t.border, width: t.bw),
              boxShadow: t.shadow,
            ),
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 0.75,
              vertical: t.sp * 0.5,
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final action in vaultActions(context, ref, uri))
                  _Slot(action: action),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _Slot extends StatelessWidget {
  const _Slot({required this.action});

  final VaultAction action;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;

    // The primary action is a filled circle standing proud of the pill, not a
    // sixth equal icon: on a notes app "write something" is not one option
    // among six.
    if (action.primary) {
      return Tooltip(
        message: action.tooltip,
        child: InkWell(
          onTap: action.onTap,
          customBorder: const CircleBorder(),
          child: Container(
            width: t.sp * 5.75,
            height: t.sp * 5.75,
            alignment: Alignment.center,
            margin: EdgeInsets.symmetric(horizontal: t.sp * 0.5),
            decoration: BoxDecoration(color: t.accent, shape: BoxShape.circle),
            child: StormIcon(action.glyph, size: t.sp * 2.5, color: t.onAccent),
          ),
        ),
      );
    }

    return Tooltip(
      message: action.tooltip,
      child: InkWell(
        onTap: action.onTap,
        customBorder: const CircleBorder(),
        child: Padding(
          padding: EdgeInsets.all(t.sp * 1.25),
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              StormIcon(
                action.glyph,
                size: t.sp * 2.5,
                color: action.selected ? t.accent : t.text2,
              ),
              // Amber, and carrying the count. Mentions are the one slot whose
              // value is a number, and a bare dot threw that away.
              if ((action.badge ?? 0) > 0)
                Positioned(
                  right: -6,
                  top: -6,
                  child: Container(
                    padding: EdgeInsets.symmetric(
                      horizontal: t.sp * 0.5,
                      vertical: t.sp * 0.125,
                    ),
                    constraints: BoxConstraints(minWidth: t.labelSize * 1.6),
                    decoration: BoxDecoration(
                      color: t.amber,
                      borderRadius: BorderRadius.circular(999),
                    ),
                    child: Text(
                      '${action.badge}',
                      textAlign: TextAlign.center,
                      style: TextStyle(
                        fontFamily: StormTokens.monoFamily,
                        // The 11px floor applies here too: a count nobody can
                        // read is not a count.
                        fontSize: t.labelSize,
                        height: 1.2,
                        fontWeight: FontWeight.w600,
                        color: t.bg,
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Lets the shell hand the bubble an action without the bubble knowing which
/// screen it is sitting on.
class NewNoteRequest extends InheritedWidget {
  const NewNoteRequest({
    super.key,
    required this.onRequest,
    required super.child,
  });

  final VoidCallback onRequest;

  static VoidCallback? of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<NewNoteRequest>()?.onRequest;

  @override
  bool updateShouldNotify(NewNoteRequest old) => false;
}

/// Same, for creating a folder. Absent on screens where there is no folder to
/// create one inside, which is what hides the slot.
class NewFolderRequest extends InheritedWidget {
  const NewFolderRequest({
    super.key,
    required this.onRequest,
    required super.child,
  });

  final VoidCallback onRequest;

  static VoidCallback? of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<NewFolderRequest>()?.onRequest;

  @override
  bool updateShouldNotify(NewFolderRequest old) => false;
}

/// Same, for the Context slot inside a note.
class NoteContextRequest extends InheritedWidget {
  const NoteContextRequest({
    super.key,
    required this.onRequest,
    required super.child,
  });

  final VoidCallback onRequest;

  static VoidCallback? of(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<NoteContextRequest>()
      ?.onRequest;

  @override
  bool updateShouldNotify(NoteContextRequest old) => false;
}
