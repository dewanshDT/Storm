import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
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
/// Four slots — Directory, Search, New note, Context — collapsed to a single
/// icon until tapped. Tap expands, matching the app's one interaction rule:
/// tap is the primary action, long-press is secondary options, everywhere.
///
/// Expansion is widget-local state. It is deliberately *not* a Riverpod
/// provider: the global providers here drive sync and cache, and pushing
/// ephemeral UI state into them is how the editor once got torn down and
/// rebuilt mid-open.
class NavBubble extends ConsumerStatefulWidget {
  const NavBubble({super.key});

  @override
  ConsumerState<NavBubble> createState() => _NavBubbleState();
}

class _NavBubbleState extends ConsumerState<NavBubble> {
  bool _expanded = false;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final uri = GoRouterState.of(context).uri;

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
              child: _expanded
                  ? Row(mainAxisSize: MainAxisSize.min, children: _slots(uri))
                  : _Slot(
                      icon: Icons.more_horiz,
                      tooltip: 'Navigation',
                      onTap: () => setState(() => _expanded = true),
                    ),
            ),
          ),
        ),
      ),
    );
  }

  List<Widget> _slots(Uri uri) {
    void go(String location) {
      setState(() => _expanded = false);
      context.go(location);
    }

    return [
      _Slot(
        icon: Icons.folder_outlined,
        tooltip: 'Directory',
        selected: uri.path.startsWith(Routes.browse),
        onTap: () => go(Routes.browse),
      ),
      _Slot(
        icon: Icons.search,
        tooltip: 'Search',
        selected: uri.path == Routes.search,
        onTap: () => go(Routes.search),
      ),
      _Slot(
        icon: Icons.add,
        tooltip: 'New note',
        onTap: () {
          setState(() => _expanded = false);
          NewNoteRequest.of(context)?.call();
        },
      ),
      _ContextSlot(uri: uri, onGo: go),
      _Slot(
        icon: Icons.close,
        tooltip: 'Close',
        onTap: () => setState(() => _expanded = false),
      ),
    ];
  }
}

/// The dynamic fourth slot.
///
/// Its meaning follows the router, not a separately maintained flag: the tag
/// browser at the vault root, and the open note's linked mentions inside a
/// note. One source of truth for "where am I".
class _ContextSlot extends ConsumerWidget {
  const _ContextSlot({required this.uri, required this.onGo});

  final Uri uri;
  final void Function(String) onGo;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final inNote = uri.pathSegments.firstOrNull == 'note';

    if (!inNote) {
      return _Slot(
        icon: Icons.label_outline,
        tooltip: 'Tags',
        selected: uri.path == Routes.tags,
        onTap: () => onGo(Routes.tags),
      );
    }

    final id = uri.pathSegments.length > 1 ? uri.pathSegments[1] : null;
    final mentions = id == null
        ? 0
        : (ref.watch(backlinksProvider(id)).value?.length ?? 0);

    return _Slot(
      icon: Icons.hub_outlined,
      tooltip: mentions == 1 ? '1 linked mention' : '$mentions linked mentions',
      badge: mentions > 0,
      onTap: () => NoteContextRequest.of(context)?.call(),
    );
  }
}

class _Slot extends StatelessWidget {
  const _Slot({
    required this.icon,
    required this.tooltip,
    required this.onTap,
    this.selected = false,
    this.badge = false,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;
  final bool selected;
  final bool badge;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Tooltip(
      message: tooltip,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(21),
        child: Padding(
          padding: const EdgeInsets.all(11),
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Icon(
                icon,
                size: 22,
                color: selected ? scheme.primary : scheme.onSurfaceVariant,
              ),
              if (badge)
                Positioned(
                  right: -2,
                  top: -2,
                  child: StormStatusDot(
                    status: StormStatus.syncing,
                    size: 7,
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

  static VoidCallback? of(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<NewNoteRequest>()
      ?.onRequest;

  @override
  bool updateShouldNotify(NewNoteRequest old) => false;
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
