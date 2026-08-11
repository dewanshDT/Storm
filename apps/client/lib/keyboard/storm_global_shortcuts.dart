import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../state/app_state.dart' show sidebarCollapsedProvider;
import '../ui/breakpoints.dart';
import '../ui/shell/nav_bubble.dart' show NewFolderRequest, NewNoteRequest;
import '../ui/shell/vault_gate.dart';
import 'storm_activators.dart' show stormHasKeyboard;
import 'storm_intents.dart';
import 'storm_shortcut_maps.dart';

/// Global chords: search, new note/folder, sidebar toggle.
///
/// Lives under [NewNoteRequest] / [NewFolderRequest] so N / ⇧N hit the same
/// callbacks as the nav pill. Touch layouts are unaffected — [Shortcuts] only
/// fires on key events.
class StormGlobalShortcuts extends ConsumerWidget {
  const StormGlobalShortcuts({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Focus so chords work when no TextField is focused (e.g. Read Mode,
    // browse). skipTraversal keeps Tab order on real fields. Autofocus only
    // where a physical keyboard exists: on a phone the shell grabbing primary
    // focus arms nothing and would fight a field that autofocuses on entry.
    return Focus(
      autofocus: stormHasKeyboard,
      skipTraversal: true,
      child: Shortcuts(
        shortcuts: stormGlobalShortcuts(),
        child: Actions(
          actions: {
            StormSearchIntent: CallbackAction<StormSearchIntent>(
              onInvoke: (_) {
                final vaultId = VaultGate.maybeOf(context);
                if (vaultId == null || vaultId.isEmpty) return null;
                context.go(Routes.search(vaultId));
                return null;
              },
            ),
            StormNewNoteIntent: CallbackAction<StormNewNoteIntent>(
              onInvoke: (_) {
                NewNoteRequest.of(context)?.call();
                return null;
              },
            ),
            StormNewFolderIntent: CallbackAction<StormNewFolderIntent>(
              onInvoke: (_) {
                NewFolderRequest.of(context)?.call();
                return null;
              },
            ),
            StormToggleSidebarIntent: CallbackAction<StormToggleSidebarIntent>(
              onInvoke: (_) {
                if (!context.isExpanded) return null;
                final collapsed = ref.read(sidebarCollapsedProvider);
                ref.read(sidebarCollapsedProvider.notifier).state = !collapsed;
                return null;
              },
            ),
          },
          child: child,
        ),
      ),
    );
  }
}
