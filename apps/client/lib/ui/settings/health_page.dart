import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../agent/agent_state.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/client_version.dart';
import '../../state/health.dart';
import '../controls.dart' show SettingsRow;
import '../report_sheet.dart' show showReportProblemSheet;
import '../shell/app_bar.dart' show healthColor;
import '../tokens.dart';
import '../widgets.dart' show KeyChip;
import 'connection_page.dart' show serverAddress;
import 'feedback_toggle.dart' show FeedbackToggleRow;
import 'settings_widgets.dart';
import 'theme_picker.dart' show ThemeCredits;

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
    final channel = ref.watch(stormChannelProvider);

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
        GroupLabel('Feedback', top: t.sp * 2.5),
        const FeedbackToggleRow(),
        SettingsRow(
          key: const Key('report-problem'),
          label: 'Report a problem',
          sub: 'Send what happened, with diagnostics you can see first.',
          divider: false,
          trailing: Icon(
            LucideIcons.chevron_right,
            size: t.codeSize,
            color: t.text2,
          ),
          onTap: () => showReportProblemSheet(context),
        ),
        Padding(
          padding: EdgeInsets.only(top: t.sp * 2),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              Flexible(
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
              // Decision 163: nothing extra on a stable build — the public
              // line is the assumed default, and only the other one needs
              // calling out.
              if (channel == StormChannel.latest) ...[
                SizedBox(width: t.sp * 0.75),
                const KeyChip(
                  key: Key('about-channel-latest'),
                  label: 'Latest',
                ),
              ],
            ],
          ),
        ),
        const ThemeCredits(),
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
