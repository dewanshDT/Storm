import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import '../states.dart';
import 'storm_scaffold.dart';

/// The Notes activity with nowhere remembered: opens the active vault (or
/// the first one), or says there are none yet (handoff §1.5).
class NotesHome extends ConsumerStatefulWidget {
  const NotesHome({super.key});

  @override
  ConsumerState<NotesHome> createState() => _NotesHomeState();
}

class _NotesHomeState extends ConsumerState<NotesHome> {
  bool _sent = false;

  void _open(String vaultId) {
    if (_sent) return;
    _sent = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) context.go(Routes.browse(vaultId));
    });
  }

  @override
  Widget build(BuildContext context) {
    final vaults = ref.watch(vaultsProvider);
    final activeId = ref.watch(activeVaultProvider);

    return StormScaffold(
      showNav: false,
      child: vaults.when(
        loading: () => const Center(child: CircularProgressIndicator()),
        error: (e, _) => EmptyState(
          icon: LucideIcons.cloud_off,
          title: "Couldn't list your vaults",
          detail: describeFailure(e),
          fill: true,
        ),
        data: (list) {
          final usable = list.where((v) => !v.missing).toList();
          if (usable.isEmpty) {
            return EmptyState(
              icon: LucideIcons.folder_plus,
              title: 'No vaults yet',
              detail: 'A vault is a folder of notes on your Storm.',
              action: 'New vault',
              onAction: () => context.go(Routes.settingsPage('vaults')),
              fill: true,
            );
          }
          final target =
              usable.where((v) => v.id == activeId).firstOrNull ?? usable.first;
          _open(target.id);
          return const SizedBox.shrink();
        },
      ),
    );
  }
}
