import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/app_state.dart';

/// Makes the route's vault the active one before its screens are built.
///
/// The route is the source of truth for which vault is open; `Settings.
/// activeVault` is a persisted mirror that `apiProvider` and the sync engine
/// read. Between navigating and the settings write landing there is one frame
/// where the two disagree — and in that frame `treeProvider` still holds the
/// *previous* vault's notes. Rendering it would show one vault's notes under
/// another's name.
///
/// So this refuses to build its child until they agree. That is the whole job:
/// a spinner for one frame is not a loading state, it is the absence of a lie.
///
/// The write happens after the frame because mutating a provider during build
/// is not allowed — the same reason `NoteScreen._load` defers.
class VaultGate extends ConsumerStatefulWidget {
  const VaultGate({super.key, required this.vaultId, required this.child});

  final String vaultId;
  final Widget child;

  /// The vault the enclosing route is for.
  ///
  /// Widgets below take the vault from here rather than from settings, so they
  /// cannot read a value the gate has not accepted yet.
  static String of(BuildContext context) {
    final scope = context.dependOnInheritedWidgetOfExactType<_VaultScope>();
    assert(scope != null, 'VaultGate.of called outside a VaultGate');
    return scope!.vaultId;
  }

  @override
  ConsumerState<VaultGate> createState() => _VaultGateState();
}

class _VaultGateState extends ConsumerState<VaultGate> {
  @override
  void initState() {
    super.initState();
    _adopt();
  }

  @override
  void didUpdateWidget(VaultGate old) {
    super.didUpdateWidget(old);
    if (old.vaultId != widget.vaultId) _adopt();
  }

  void _adopt() {
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      if (!mounted) return;
      final settings = ref.read(settingsProvider).value;
      if (settings == null || settings.activeVault == widget.vaultId) return;
      await ref
          .read(settingsProvider.notifier)
          .save(settings.copyWith(activeVault: widget.vaultId));
    });
  }

  @override
  Widget build(BuildContext context) {
    final active = ref.watch(activeVaultProvider);
    if (active != widget.vaultId) {
      return const Scaffold(body: Center(child: CircularProgressIndicator()));
    }
    return _VaultScope(vaultId: widget.vaultId, child: widget.child);
  }
}

class _VaultScope extends InheritedWidget {
  const _VaultScope({required this.vaultId, required super.child});

  final String vaultId;

  @override
  bool updateShouldNotify(_VaultScope old) => old.vaultId != vaultId;
}
