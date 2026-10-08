import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../breakpoints.dart';
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

/// The phone pill: Directory, Search, New note and Tags. The note screen
/// hides it (Q5); the sidebar footer carries the same actions at desk width.
class NavBubble extends ConsumerWidget {
  const NavBubble({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final uri = GoRouterState.of(context).uri;

    if (context.isExpanded) return const SizedBox.shrink();

    return SafeArea(
      minimum: EdgeInsets.only(bottom: t.sp * 3.25),
      child: Align(
        alignment: Alignment.bottomCenter,
        child: StormPill(
          padding: EdgeInsets.all(t.sp * 0.75),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            spacing: t.sp * 0.5,
            children: [
              for (final action in vaultActions(context, ref, uri))
                _Slot(action: action),
            ],
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

    if (action.primary) {
      return Tooltip(
        message: action.tooltip,
        child: InkWell(
          onTap: action.onTap,
          onLongPress: action.onLongPress,
          customBorder: const CircleBorder(),
          child: PrimaryCircle(icon: action.icon, size: t.sp * 6),
        ),
      );
    }

    return Tooltip(
      message: action.tooltip,
      child: InkWell(
        onTap: action.onTap,
        customBorder: const CircleBorder(),
        child: SizedBox(
          width: t.sp * 5.5,
          height: t.sp * 5.5,
          child: Center(
            child: Icon(action.icon, size: t.sp * 2.375, color: t.text2),
          ),
        ),
      ),
    );
  }
}

/// The surface every floating action bar is drawn on: the nav bubble, and
/// the Agents space's *New session* (decision 78). One widget, so the two
/// cannot drift apart.
class StormPill extends StatelessWidget {
  const StormPill({super.key, required this.child, this.padding});

  final Widget child;

  /// Defaults to the nav bubble's 5px, where the slots are the padding.
  final EdgeInsetsGeometry? padding;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      decoration: BoxDecoration(
        color: t.surface.withValues(alpha: 0.96),
        // A pill, where the corner bubbles are rounded squares. The shape
        // difference is deliberate grammar: corners are places, the pill is
        // an action bar.
        borderRadius: BorderRadius.circular(999),
        border: Border.all(color: t.border, width: t.bw),
        boxShadow: t.shadow,
      ),
      // 5px all round, as the prototype has it. A wider inset made the pill
      // read as a bar.
      padding: padding ?? EdgeInsets.all(t.sp * 0.625),
      child: child,
    );
  }
}

/// The primary action: a filled accent circle standing proud of the pill.
class PrimaryCircle extends StatelessWidget {
  const PrimaryCircle({super.key, required this.icon, this.size});

  final IconData icon;
  final double? size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      width: size ?? t.sp * 5.5,
      height: size ?? t.sp * 5.5,
      alignment: Alignment.center,
      decoration: BoxDecoration(color: t.accent, shape: BoxShape.circle),
      child: Icon(icon, size: t.sp * 2.75, color: t.onAccent),
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
