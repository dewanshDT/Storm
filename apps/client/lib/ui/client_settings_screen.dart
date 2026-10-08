import 'package:flutter/material.dart';

import 'shell/corner_bubbles.dart' show ClientSettingsBody;
import 'settings/settings_shell.dart' show settingsLeading;
import 'tokens.dart';

/// Settings › This device.
class ClientSettingsScreen extends StatelessWidget {
  const ClientSettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;

    return Scaffold(
      appBar: AppBar(
        leading: settingsLeading(context),
        automaticallyImplyLeading: false,
        title: const Text('This device'),
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
