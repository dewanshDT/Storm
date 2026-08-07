import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../breakpoints.dart';
import 'vault_actions.dart';
import '../theme.dart';

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
    final scheme = Theme.of(context).colorScheme;
    final uri = GoRouterState.of(context).uri;

    // The sidebar carries these on a wide screen.
    if (context.isExpanded) return const SizedBox.shrink();

    return SafeArea(
      minimum: const EdgeInsets.only(bottom: 16),
      child: Align(
        alignment: Alignment.bottomCenter,
        child: AnimatedSize(
          duration: const Duration(milliseconds: 180),
          curve: Curves.easeOutCubic,
          child: Material(
            color: scheme.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(26),
            elevation: 6,
            shadowColor: Colors.black.withValues(alpha: 0.4),
            child: Padding(
              padding: const EdgeInsets.all(5),
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
      ),
    );
  }
}

class _Slot extends StatelessWidget {
  const _Slot({required this.action});

  final VaultAction action;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Tooltip(
      message: action.tooltip,
      child: InkWell(
        onTap: action.onTap,
        borderRadius: BorderRadius.circular(21),
        child: Padding(
          padding: const EdgeInsets.all(11),
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Icon(
                action.icon,
                size: 22,
                color: action.selected
                    ? scheme.primary
                    : scheme.onSurfaceVariant,
              ),
              if ((action.badge ?? 0) > 0)
                const Positioned(
                  right: -2,
                  top: -2,
                  child: StormStatusDot(status: StormStatus.syncing, size: 7),
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
