import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../agent/hosts_screen.dart';
import '../../agent/integrations_screen.dart';
import '../../router.dart';
import '../../state/app_state.dart' show serverConfigProvider;
import '../../state/health.dart' show integrationsAttentionProvider;
import '../breakpoints.dart';
import '../controls.dart';
import '../shell/sidebar_frame.dart';
import '../shell/storm_scaffold.dart';
import '../tokens.dart';
import 'access_page.dart';
import 'advanced_page.dart';
import 'ai_page.dart';
import 'connection_page.dart';
import 'device_page.dart';
import 'health_page.dart';
import 'settings_widgets.dart';
import 'storage_page.dart';
import 'vaults_page.dart';

/// One settings page (handoff §5.1). [storm] pages sit under the STORM label.
class SettingsPageInfo {
  const SettingsPageInfo(this.id, this.label, {this.storm = false});

  final String id;
  final String label;
  final bool storm;
}

const kSettingsPages = [
  SettingsPageInfo('device', 'This device'),
  SettingsPageInfo('access', 'Devices & access'),
  SettingsPageInfo('vaults', 'Vaults', storm: true),
  SettingsPageInfo('ai', 'AI access', storm: true),
  SettingsPageInfo('integrations', 'Integrations', storm: true),
  SettingsPageInfo('hosts', 'Hosts & default agent', storm: true),
  SettingsPageInfo('storage', 'Storage', storm: true),
  SettingsPageInfo('connection', 'Connection', storm: true),
  SettingsPageInfo('advanced', 'Advanced', storm: true),
  SettingsPageInfo('health', 'About & health'),
];

bool isSettingsPage(String id) => kSettingsPages.any((p) => p.id == id);

Widget settingsPageFor(String id) =>
    _FreshConfig(key: ValueKey(id), child: _pageFor(id));

Widget _pageFor(String id) => switch (id) {
  'access' => const AccessPage(),
  'vaults' => const VaultsPage(),
  'ai' => const AiPage(),
  'integrations' => const IntegrationsScreen(),
  'hosts' => const HostsScreen(),
  'storage' => const StoragePage(),
  'connection' => const ConnectionPage(),
  'advanced' => const AdvancedPage(),
  'health' => const HealthPage(),
  _ => const DevicePage(),
};

/// Re-reads the server's settings each time a page opens: another device
/// may have changed them since this one last asked.
class _FreshConfig extends ConsumerStatefulWidget {
  const _FreshConfig({super.key, required this.child});

  final Widget child;

  @override
  ConsumerState<_FreshConfig> createState() => _FreshConfigState();
}

class _FreshConfigState extends ConsumerState<_FreshConfig> {
  @override
  void initState() {
    super.initState();
    Future.microtask(() {
      if (mounted) ref.invalidate(serverConfigProvider);
    });
  }

  @override
  Widget build(BuildContext context) => widget.child;
}

/// The danger dot beside Integrations while a connection needs attention.
class _AttentionDot extends ConsumerWidget {
  const _AttentionDot();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    if (!ref.watch(integrationsAttentionProvider)) return const SizedBox();
    return Container(
      key: const Key('integrations-attention'),
      width: t.sp * 0.75,
      height: t.sp * 0.75,
      decoration: BoxDecoration(shape: BoxShape.circle, color: t.danger),
    );
  }
}

/// Settings navigation beside the page at desk width; on a phone the list
/// and each page are separate screens.
class SettingsShell extends StatelessWidget {
  const SettingsShell({super.key, required this.page, required this.child});

  final String? page;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (!context.isExpanded) return child;
    final selected = page ?? 'device';
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SidebarFrame(
          body: SettingsNav(
            selected: selected,
            onOpen: (id) => context.go(Routes.settingsPage(id)),
          ),
        ),
        Expanded(
          child: PaneSemantics(
            child: page == null ? settingsPageFor(selected) : child,
          ),
        ),
      ],
    );
  }
}

class SettingsNav extends StatelessWidget {
  const SettingsNav({super.key, required this.selected, required this.onOpen});

  final String? selected;
  final ValueChanged<String> onOpen;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    Widget item(SettingsPageInfo p) {
      final on = p.id == selected;
      return Material(
        color: on ? t.surface2 : Colors.transparent,
        borderRadius: BorderRadius.circular(t.rControl),
        child: InkWell(
          key: Key('settings-nav-${p.id}'),
          borderRadius: BorderRadius.circular(t.rControl),
          onTap: () => onOpen(p.id),
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.25,
              vertical: t.sp * 0.875,
            ),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    p.label,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: on ? t.text : t.text2,
                    ),
                  ),
                ),
                if (p.id == 'integrations') const _AttentionDot(),
              ],
            ),
          ),
        ),
      );
    }

    return ListView(
      padding: EdgeInsets.fromLTRB(t.sp * 1.5, t.sp * 2.75, t.sp * 1.5, t.sp),
      children: [
        Padding(
          padding: EdgeInsets.only(left: t.sp * 1.25, bottom: t.sp * 1.5),
          child: Text(
            'Settings',
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.uiSize,
              fontWeight: FontWeight.w600,
              color: t.text,
            ),
          ),
        ),
        for (final p in kSettingsPages.where(
          (p) => !p.storm && p.id != 'health',
        ))
          item(p),
        Padding(
          padding: EdgeInsets.fromLTRB(t.sp * 1.25, t.sp * 2, 0, t.sp * 0.75),
          child: const GroupLabel('Storm', bottom: 0),
        ),
        for (final p in kSettingsPages.where((p) => p.storm)) item(p),
        item(kSettingsPages.last),
      ],
    );
  }
}

/// The phone's settings list, reached from the right corner bubble.
class SettingsListScreen extends StatelessWidget {
  const SettingsListScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    Widget row(SettingsPageInfo p) => SettingsRow(
      key: Key('settings-row-${p.id}'),
      label: p.label,
      large: true,
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        spacing: t.sp,
        children: [
          if (p.id == 'integrations') const _AttentionDot(),
          Icon(LucideIcons.chevron_right, size: t.uiSize, color: t.text3),
        ],
      ),
      onTap: () => context.push(Routes.settingsPage(p.id)),
    );

    return StormScaffold(
      showNav: false,
      child: ListView(
        padding: EdgeInsets.symmetric(horizontal: t.cardPad),
        children: [
          Text(
            'Settings',
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.titleSize,
              fontWeight: FontWeight.w600,
              color: t.text,
            ),
          ),
          SizedBox(height: t.sp),
          for (final p in kSettingsPages.where(
            (p) => !p.storm && p.id != 'health',
          ))
            row(p),
          GroupLabel('Storm', top: t.sp * 3),
          for (final p in kSettingsPages.where((p) => p.storm)) row(p),
          row(kSettingsPages.last),
        ],
      ),
    );
  }
}
