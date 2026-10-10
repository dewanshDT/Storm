import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../agent/agent_state.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/client_version.dart';
import '../../state/health.dart';
import '../shell/activity_rail.dart' show healthColor;
import '../tokens.dart';
import 'connection_page.dart' show serverAddress;
import 'settings_widgets.dart';

/// Settings › About & health: the rail popover's rows, each with the place
/// that fixes it, then what this is and where it points.
class HealthPage extends ConsumerWidget {
  const HealthPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final rows = ref.watch(healthRowsProvider);
    final overview = ref.watch(agentOverviewProvider).value;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    // Both, labelled: a test build reports its own version, which need not
    // be the server's (B-7).
    final serverVersion = ref.watch(serverConfigProvider).value?.version;
    final appVersion = ref.watch(clientVersionProvider).value;

    Future<void> syncNow() async {
      await ref.read(syncEngineProvider).sync();
      ref.invalidate(treeProvider);
    }

    final noHosts =
        overview != null && !overview.unreachable && overview.hosts.isEmpty;

    return SettingsPage(
      title: 'About & health',
      intro: 'Whether everything is working.',
      children: [
        for (final (i, row) in rows.indexed) ...[
          _HealthLine(
            row: row,
            action: i == 0
                ? 'Sync now'
                : switch (row.route) {
                    '/settings/hosts' => 'Hosts',
                    '/settings/integrations' => 'Integrations',
                    _ => null,
                  },
            onAction: i == 0
                ? syncNow
                : row.route == null
                ? null
                : () => context.go(row.route!),
          ),
          if (i == 0 && noHosts)
            _HealthLine(
              row: const HealthRow(HealthTone.muted, 'No hosts enrolled'),
              action: 'Enroll',
              onAction: () => context.go(Routes.settingsPage('hosts')),
            ),
        ],
        Padding(
          padding: EdgeInsets.only(top: t.sp * 2),
          child: Text(
            [
              'App ${appVersion ?? '?'}',
              if (serverVersion != null) 'Server $serverVersion',
              serverAddress(settings.baseUrl),
            ].join(' · '),
            key: const Key('about-footer'),
            style: TextStyle(
              fontFamily: StormTokens.monoFamily,
              fontSize: t.labelSize * 1.09,
              color: t.text3,
            ),
          ),
        ),
      ],
    );
  }
}

class _HealthLine extends StatelessWidget {
  const _HealthLine({required this.row, this.action, this.onAction});

  final HealthRow row;
  final String? action;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(vertical: t.sp * 1.625),
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Row(
        children: [
          Container(
            width: t.sp,
            height: t.sp,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: healthColor(row.tone, t),
            ),
          ),
          SizedBox(width: t.sp * 1.5),
          Expanded(
            child: Text(
              row.text,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.uiSize,
                color: t.text,
              ),
            ),
          ),
          if (action != null && onAction != null)
            TextAction(label: action!, accent: true, onTap: onAction),
        ],
      ),
    );
  }
}
