import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import 'accents.dart';
import 'controls.dart';
import 'session_status.dart';
import 'settings/settings_widgets.dart' show SettingsDivider;
import 'states.dart';
import 'theme.dart';
import 'tokens.dart';
import 'widgets.dart';

/// Every shared widget, in all three themes at once.
///
/// A judgement surface, not a feature: the token layer only pays for itself if
/// one change can be seen moving everything, and three columns side by side is
/// the only way to catch a colour that works in one identity and not another.
class GalleryScreen extends StatelessWidget {
  const GalleryScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: const Color(0xFF15130F),
      body: SafeArea(
        child: LayoutBuilder(
          builder: (context, box) => SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: SizedBox(
              height: box.maxHeight,
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final preset in StormPreset.values)
                    SizedBox(
                      width: 380,
                      child: Theme(
                        data: StormTheme.from(preset),
                        child: const _Column(),
                      ),
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _Column extends StatelessWidget {
  const _Column();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return ColoredBox(
      color: t.bg,
      child: SingleChildScrollView(
        padding: EdgeInsets.all(t.cardPad),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              t.variant.label,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.headingSize,
                fontWeight: FontWeight.w600,
                color: t.text,
              ),
            ),
            SizedBox(height: t.gap),

            _Card(
              label: 'brand · status · save state',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const BrandMark(size: 44, withWordmark: true),
                  SizedBox(height: t.gap),
                  Row(
                    children: [
                      for (final s in DotStatus.values) ...[
                        StatusDot(status: s),
                        SizedBox(width: t.gap),
                      ],
                    ],
                  ),
                  SizedBox(height: t.gap),
                  const SaveStateLabel(label: 'Saved', tone: SaveTone.good),
                  const SaveStateLabel(
                    label: 'Saving…',
                    tone: SaveTone.working,
                  ),
                  const SaveStateLabel(
                    label: 'Queued — offline',
                    tone: SaveTone.waiting,
                  ),
                  const SaveStateLabel(label: 'Failed', tone: SaveTone.bad),
                ],
              ),
            ),

            _Card(
              label: 'chips',
              child: Wrap(
                spacing: t.sp,
                runSpacing: t.sp,
                children: const [
                  TagChip(label: '#proj/storm'),
                  TagChip(label: '#cooking'),
                  KeyChip(label: 'status'),
                  KeyChip(label: 'created'),
                ],
              ),
            ),

            _Card(
              label: 'accents',
              child: AccentPicker(selected: Accent.sage, onSelected: (_) {}),
            ),

            _Card(
              label: 'controls',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const StormInput(hintText: 'Search notes'),
                  SizedBox(height: t.gap),
                  Row(
                    children: [
                      StormToggle(value: true, onChanged: (_) {}),
                      SizedBox(width: t.gap),
                      StormToggle(value: false, onChanged: (_) {}),
                      SizedBox(width: t.gap),
                      StormCheckbox(value: true, onChanged: (_) {}),
                      SizedBox(width: t.gap),
                      StormCheckbox(value: false, onChanged: (_) {}),
                    ],
                  ),
                  SizedBox(height: t.gap),
                  Wrap(
                    spacing: t.sp,
                    runSpacing: t.sp,
                    children: [
                      FilledButton(
                        onPressed: () {},
                        child: const Text('Connect'),
                      ),
                      OutlinedButton(
                        onPressed: () {},
                        child: const Text('Cancel'),
                      ),
                      TextButton(
                        onPressed: () {},
                        child: const Text('Sync now'),
                      ),
                    ],
                  ),
                ],
              ),
            ),

            _Card(
              label: 'v2 · multiline input · image thumbnail',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const StormInput(
                    hintText: 'What happened?',
                    minLines: 3,
                    maxLines: 6,
                  ),
                  SizedBox(height: t.gap),
                  ImageThumbnail(
                    image: const AssetImage('assets/icon/storm_icon_full.png'),
                    onRemove: () {},
                  ),
                ],
              ),
            ),

            _Card(
              label: 'v2 · buttons',
              child: Wrap(
                spacing: t.sp,
                runSpacing: t.sp,
                children: [
                  StormButton.primary(label: '＋ New vault', onPressed: () {}),
                  StormButton.outline(
                    label: '＋ Add a device',
                    onPressed: () {},
                  ),
                  StormButton.soft(
                    label: 'Start session',
                    icon: LucideIcons.play,
                    onPressed: () {},
                  ),
                  StormButton.danger(label: 'End session', onPressed: () {}),
                  StormButton.text(label: 'Revoke', onPressed: () {}),
                  const StormButton.primary(label: 'Disabled', onPressed: null),
                ],
              ),
            ),

            _Card(
              label: 'v2 · choice chips · settings rows',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  ChoiceChips<String>(
                    options: const [
                      ChoiceOption('claude', 'Claude Code'),
                      ChoiceOption('opencode', 'OpenCode'),
                      ChoiceOption('shell', 'Shell'),
                    ],
                    selected: 'claude',
                    onSelected: (_) {},
                  ),
                  SizedBox(height: t.gap),
                  SettingsRow(
                    label: 'Let AI apps read your notes',
                    sub: 'Serves your vaults to apps that hold an access key.',
                    trailing: StormToggle(value: true, onChanged: (_) {}),
                  ),
                  SettingsRow(
                    label: 'Let AI apps write',
                    sub: 'Edits merge like any other device.',
                    trailing: StormToggle(value: false, onChanged: (_) {}),
                  ),
                  SettingsRow(
                    label: 'laptop-claude',
                    sub: 'stk_••••3f9a · used 1h ago',
                    monoSub: true,
                    trailing: StormButton.text(
                      label: 'Revoke',
                      onPressed: () {},
                    ),
                  ),
                  SizedBox(height: t.gap),
                  const SettingsDivider(),
                  SizedBox(height: t.gap),
                  SettingsRow(
                    label: 'Below a settings divider',
                    divider: false,
                  ),
                ],
              ),
            ),

            _Card(
              label: 'v2 · choice cards',
              child: ChoiceCards<String>(
                key: const Key('gallery-choice-cards'),
                options: const [
                  ChoiceCardOption('storm', 'Storm', subtitle: 'Default'),
                  ChoiceCardOption(
                    'catppuccin',
                    'Catppuccin',
                    subtitle: 'Mocha',
                    note: 'Inspired by Catppuccin',
                  ),
                  ChoiceCardOption(
                    'slowflow',
                    'SlowFlow',
                    subtitle: 'Earth',
                    note: 'Light only',
                  ),
                  ChoiceCardOption(
                    'off',
                    'Unavailable',
                    subtitle: 'Disabled',
                    enabled: false,
                  ),
                ],
                selected: 'catppuccin',
                onSelected: (_) {},
                minCardWidth: 120,
              ),
            ),

            _Card(
              label: 'v2 · session status',
              child: Wrap(
                spacing: t.sp,
                runSpacing: t.sp,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  for (final s in const [
                    'starting',
                    'running',
                    'unknown',
                    'completed',
                    'stopped',
                    'failed',
                  ]) ...[
                    SessionStatusDot(status: s),
                    SessionStatusChip(status: s),
                  ],
                ],
              ),
            ),

            _Card(
              label: 'v2 · numbered steps',
              child: NumberedSteps(
                steps: [
                  NumberedStep(
                    'Enroll a host',
                    action: StormButton.primary(
                      label: 'Enroll a host',
                      onPressed: () {},
                    ),
                  ),
                  const NumberedStep(
                    'Sign Claude Code or OpenCode in on that host',
                  ),
                  const NumberedStep('Start a session'),
                ],
              ),
            ),

            _Card(
              label: 'rows',
              child: Column(
                children: [
                  const NoteRow(
                    title: 'Sprint Retro',
                    meta: 'Projects · 2h ago',
                    divider: true,
                  ),
                  const FolderRow(name: 'Projects', count: 2, divider: true),
                  SizedBox(height: t.sp),
                  Breadcrumb(
                    crumbs: [
                      Crumb('Vaults', onTap: () {}),
                      Crumb('Work', onTap: () {}),
                      const Crumb('Projects'),
                    ],
                  ),
                  SizedBox(height: t.sp),
                  const StatusBar(
                    version: 12,
                    label: 'Saved',
                    tone: SaveTone.good,
                  ),
                  SizedBox(height: t.sp),
                  const PopoverItem(label: 'Work', subtitle: '14 notes'),
                ],
              ),
            ),

            _Card(
              label: 'tags · stats · vault card',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const TagGroup(
                    name: 'proj',
                    tags: ['proj/storm', 'proj/api'],
                  ),
                  SizedBox(height: t.gap),
                  Row(
                    children: [
                      const StatBlock(value: '27', label: 'notes'),
                      SizedBox(width: t.sp * 4),
                      const StatBlock(value: '2m', label: 'last synced'),
                    ],
                  ),
                  SizedBox(height: t.gap),
                  SizedBox(
                    width: 180,
                    height: 130,
                    child: VaultCard(
                      name: 'Work',
                      tile: Accent.lavender.tile(t),
                      subtitle: '14 notes',
                      status: DotStatus.synced,
                    ),
                  ),
                ],
              ),
            ),

            _Card(
              label: 'count badge',
              child: Row(
                children: [
                  CountBadge(count: 3, color: t.danger),
                  SizedBox(width: t.sp * 2),
                  CountBadge(count: 12, color: t.accent),
                ],
              ),
            ),

            _Card(
              label: 'icon badge button',
              child: Row(
                children: [
                  IconBadgeButton(
                    icon: LucideIcons.bell,
                    tooltip: 'Notifications',
                    count: 0,
                    onTap: () {},
                  ),
                  SizedBox(width: t.sp * 2),
                  IconBadgeButton(
                    icon: LucideIcons.bell,
                    tooltip: 'Notifications',
                    count: 4,
                    onTap: () {},
                  ),
                ],
              ),
            ),

            _Card(
              label: 'unread row',
              child: Column(
                children: [
                  UnreadRow(
                    unread: true,
                    onTap: () {},
                    child: Text(
                      'An unread row',
                      style: TextStyle(color: t.text),
                    ),
                  ),
                  UnreadRow(
                    unread: false,
                    onTap: () {},
                    child: Text('A read row', style: TextStyle(color: t.text)),
                  ),
                  UnreadRow(
                    unread: true,
                    onTap: () {},
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'From a session (an agent)',
                          style: TextStyle(
                            color: t.text2,
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        Text(
                          'With a source line and a held reason',
                          style: TextStyle(color: t.text),
                        ),
                        Text(
                          'Held: quiet hours',
                          style: TextStyle(color: t.text3),
                        ),
                      ],
                    ),
                  ),
                  UnreadRow(
                    unread: false,
                    onTap: null,
                    child: Text(
                      'A disabled row (no onTap)',
                      style: TextStyle(color: t.text),
                    ),
                  ),
                ],
              ),
            ),

            _Card(
              label: 'inbox header · kind badge',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  InboxHeader(
                    title: 'Notifications',
                    unreadCount: 3,
                    actions: [
                      IconBadgeButton(
                        icon: LucideIcons.check_check,
                        tooltip: 'Mark all read',
                        dimension: t.sp * 3.25,
                        onTap: () {},
                      ),
                      IconBadgeButton(
                        icon: LucideIcons.settings,
                        tooltip: 'Notification settings',
                        dimension: t.sp * 3.25,
                        onTap: () {},
                      ),
                      IconBadgeButton(
                        icon: LucideIcons.ellipsis_vertical,
                        tooltip: 'More',
                        dimension: t.sp * 3.25,
                        onTap: () {},
                      ),
                    ],
                  ),
                  SizedBox(height: t.gap),
                  Row(
                    children: [
                      KindBadge(
                        icon: LucideIcons.circle_question_mark,
                        color: t.accent,
                      ),
                      SizedBox(width: t.sp * 1.5),
                      KindBadge(
                        icon: LucideIcons.circle_alert,
                        color: t.danger,
                      ),
                      SizedBox(width: t.sp * 1.5),
                      KindBadge(
                        icon: LucideIcons.circle_check_big,
                        color: t.green,
                      ),
                      SizedBox(width: t.sp * 1.5),
                      KindBadge(icon: LucideIcons.trash_2, color: t.amber),
                      SizedBox(width: t.sp * 1.5),
                      KindBadge(icon: LucideIcons.server_off, color: t.text3),
                    ],
                  ),
                ],
              ),
            ),

            _Card(
              label: 'notification row · group',
              child: Column(
                children: [
                  NotificationRow(
                    leading: KindBadge(
                      icon: LucideIcons.circle_question_mark,
                      color: t.accent,
                    ),
                    title: [
                      TextSpan(
                        text: 'tidier',
                        style: const TextStyle(fontWeight: FontWeight.w600),
                      ),
                      const TextSpan(text: ' needs you'),
                    ],
                    subtitle: 'Claude Code',
                    unread: true,
                    time: '3m',
                    onTap: () {},
                    onMenu: (_) {},
                  ),
                  NotificationRow(
                    leading: KindBadge(
                      icon: LucideIcons.square_terminal,
                      color: t.text2,
                    ),
                    title: [
                      TextSpan(
                        text: 'tidier',
                        style: const TextStyle(fontWeight: FontWeight.w600),
                      ),
                      const TextSpan(text: ' sent a message'),
                    ],
                    subtitle: 'Codex',
                    tag: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(
                          LucideIcons.moon,
                          size: t.labelSize * 1.1,
                          color: t.text3,
                        ),
                        SizedBox(width: t.sp * 0.375),
                        Flexible(
                          child: Text(
                            'During quiet hours',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontFamily: StormTokens.sansFamily,
                              fontSize: t.labelSize * 1.09,
                              color: t.text3,
                            ),
                          ),
                        ),
                      ],
                    ),
                    body: Container(
                      margin: EdgeInsets.only(top: t.sp * 0.375),
                      padding: EdgeInsets.only(left: t.sp * 0.875),
                      decoration: BoxDecoration(
                        border: Border(
                          left: BorderSide(color: t.border, width: t.bw * 2),
                        ),
                      ),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Text(
                            'From the agent, via its terminal',
                            style: TextStyle(
                              fontFamily: StormTokens.sansFamily,
                              fontSize: t.labelSize,
                              fontWeight: FontWeight.w500,
                              color: t.text3,
                            ),
                          ),
                          SizedBox(height: t.sp * 0.25),
                          Text(
                            'tests failed on the second run',
                            style: TextStyle(
                              fontFamily: StormTokens.sansFamily,
                              fontSize: t.labelSize * 1.09,
                              color: t.text2,
                            ),
                          ),
                        ],
                      ),
                    ),
                    time: '1h',
                    onTap: () {},
                    onMenu: (_) {},
                  ),
                  NotificationGroupRow(
                    leading: KindBadge(
                      icon: LucideIcons.circle_check_big,
                      color: t.green,
                    ),
                    title: [
                      TextSpan(
                        text: 'build-vm',
                        style: const TextStyle(fontWeight: FontWeight.w600),
                      ),
                      const TextSpan(text: ' · 3 updates'),
                    ],
                    subtitle: 'latest: finished its turn · Claude Code',
                    expanded: false,
                    onToggle: () {},
                  ),
                ],
              ),
            ),

            _Card(
              label: 'states',
              child: Column(
                children: [
                  const EmptyState(
                    icon: LucideIcons.folder_open,
                    title: 'Nothing in this folder',
                    detail: 'New notes made here will land in Projects.',
                    action: 'New note',
                  ),
                  const SkeletonRows(rows: 2),
                  const OfflineNotice(queued: 2),
                  const ConflictCard(),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Card extends StatelessWidget {
  const _Card({required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(bottom: t.gap),
      child: Container(
        padding: EdgeInsets.all(t.cardPad),
        decoration: BoxDecoration(
          color: t.surface,
          borderRadius: BorderRadius.circular(t.rCard),
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SectionLabel(label),
            SizedBox(height: t.gap),
            child,
          ],
        ),
      ),
    );
  }
}
