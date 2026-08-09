import 'package:flutter/material.dart';

import 'accents.dart';
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
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: IntrinsicHeight(
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
              t.preset.label,
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
                      StormSwitch(value: true, onChanged: (_) {}),
                      SizedBox(width: t.gap),
                      StormSwitch(value: false, onChanged: (_) {}),
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
              label: 'states',
              child: Column(
                children: [
                  const EmptyState(
                    icon: Icons.folder_open,
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
