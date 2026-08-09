import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import 'shell/corner_bubbles.dart' show ClientSettingsBody;
import 'shell/vault_gate.dart';
import 'tokens.dart';

/// Theme, text size, note font — the settings that belong to this device.
///
/// "Client", not "Appearance": it also holds `Show note id` and Disconnect,
/// and the distinction that matters to the user is which side of the wire a
/// setting lives on. Server settings are the storage root, the vaults and MCP.
///
/// A page rather than a popover, because at desk width the control that opens
/// it is the sidebar's footer gear and a menu anchored there would open below
/// the bottom of the window. It is mounted inside the vault shell, so the
/// sidebar stays beside it.
class ClientSettingsScreen extends StatelessWidget {
  const ClientSettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final vaultId = VaultGate.of(context);

    return Scaffold(
      appBar: AppBar(
        leading: IconButton(
          icon: const Icon(LucideIcons.arrow_left),
          onPressed: () => context.canPop()
              ? context.pop()
              : context.go(Routes.browse(vaultId)),
        ),
        title: const Text('Client settings'),
      ),
      body: ListView(
        padding: EdgeInsets.fromLTRB(
          t.cardPad,
          t.sp,
          t.cardPad,
          t.sectionRhythm,
        ),
        children: [
          ConstrainedBox(
            constraints: BoxConstraints(maxWidth: t.sp * 60),
            child: const ClientSettingsBody(),
          ),
        ],
      ),
    );
  }
}
