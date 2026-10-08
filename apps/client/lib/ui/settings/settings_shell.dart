import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:go_router/go_router.dart';

import '../../agent/hosts_screen.dart';
import '../../agent/integrations_screen.dart';
import '../../router.dart';
import '../breakpoints.dart';
import '../client_settings_screen.dart';
import '../controls.dart';
import '../mcp_keys_screen.dart';
import '../server_settings_screen.dart';
import '../shell/activity_rail.dart' show HealthRows;
import '../shell/sidebar_frame.dart';
import '../shell/storm_scaffold.dart';
import '../tokens.dart';
import '../widgets.dart';

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

/// The page body for [id]. Until slice 7 rebuilds them, the Storm pages
/// mount the existing screens.
Widget settingsPageFor(String id) => switch (id) {
  'device' => const ClientSettingsScreen(),
  'access' => const McpKeysScreen(),
  'integrations' => const IntegrationsScreen(),
  'hosts' => const HostsScreen(),
  'health' => const _HealthPage(),
  _ => const ServerSettingsScreen(),
};

/// The back arrow a settings page shows: none at desk width, where the
/// navigation is beside it; otherwise back to the list.
Widget? settingsLeading(BuildContext context) => context.isExpanded
    ? null
    : IconButton(
        icon: const Icon(LucideIcons.arrow_left),
        tooltip: 'Settings',
        onPressed: () =>
            context.canPop() ? context.pop() : context.go(Routes.settings),
      );

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
            child: Text(
              p.label,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: on ? t.text : t.text2,
              ),
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
          child: const SectionLabel('Storm'),
        ),
        for (final p in kSettingsPages.where((p) => p.storm)) item(p),
        SizedBox(height: t.sp * 2),
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
      trailing: Icon(LucideIcons.chevron_right, size: t.uiSize, color: t.text3),
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
          Padding(
            padding: EdgeInsets.only(top: t.sp * 3, bottom: t.sp * 0.5),
            child: const SectionLabel('Storm'),
          ),
          for (final p in kSettingsPages.where((p) => p.storm)) row(p),
          row(kSettingsPages.last),
        ],
      ),
    );
  }
}

class _HealthPage extends StatelessWidget {
  const _HealthPage();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Scaffold(
      appBar: AppBar(
        leading: settingsLeading(context),
        automaticallyImplyLeading: false,
        title: const Text('About & health'),
      ),
      body: ListView(
        padding: EdgeInsets.all(t.cardPad),
        children: const [HealthRows()],
      ),
    );
  }
}
