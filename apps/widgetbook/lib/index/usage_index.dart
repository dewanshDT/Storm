import 'package:flutter/material.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/widgets.dart';
import 'package:storm_widgetbook/usage.dart';
import 'package:storm_widgetbook/usage.g.dart';
import 'package:widgetbook_annotation/widgetbook_annotation.dart' as widgetbook;

const _path = '[Index]';

@widgetbook.UseCase(name: 'shared components', type: UsageSite, path: _path)
Widget sharedUsageIndex(BuildContext context) =>
    SharedUsageTable(usage: componentUsage);

@widgetbook.UseCase(
  name: 'one-off candidates',
  type: CandidateSite,
  path: _path,
)
Widget oneOffCandidateIndex(BuildContext context) =>
    CandidateList(sites: oneOffCandidates);

/// Every shared component with where the client uses it; a component nothing
/// uses is flagged, since it is either unfinished or a candidate for removal.
class SharedUsageTable extends StatelessWidget {
  const SharedUsageTable({super.key, required this.usage});

  final Map<String, List<UsageSite>> usage;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final names = usage.keys.toList()
      ..sort((a, b) {
        final c = usage[a]!.length.compareTo(usage[b]!.length);
        return c != 0 ? c : a.compareTo(b);
      });
    final unused = names.where((n) => usage[n]!.isEmpty).length;
    return _Page(
      stats: [
        StatBlock(value: '${usage.length}', label: 'shared components'),
        StatBlock(value: '$unused', label: 'with no use'),
      ],
      children: [
        for (final name in names)
          _Group(
            title: name,
            count: usage[name]!.length,
            highlight: usage[name]!.isEmpty,
            lines: [for (final s in usage[name]!) '$s'],
            tokens: t,
          ),
      ],
    );
  }
}

/// The hand-rolled controls and surfaces still outside the shared components:
/// the migration backlog, the biggest files first.
class CandidateList extends StatelessWidget {
  const CandidateList({super.key, required this.sites});

  final List<CandidateSite> sites;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final byFile = <String, List<CandidateSite>>{};
    for (final s in sites) {
      byFile.putIfAbsent(s.file, () => []).add(s);
    }
    final files = byFile.keys.toList()
      ..sort((a, b) {
        final c = byFile[b]!.length.compareTo(byFile[a]!.length);
        return c != 0 ? c : a.compareTo(b);
      });
    return _Page(
      stats: [
        StatBlock(value: '${sites.length}', label: 'candidates'),
        StatBlock(value: '${files.length}', label: 'files'),
        for (final kind in CandidateKind.values)
          StatBlock(
            value: '${sites.where((s) => s.kind == kind).length}',
            label: kind.label,
          ),
      ],
      children: [
        for (final file in files)
          _Group(
            title: file,
            count: byFile[file]!.length,
            lines: [
              for (final s in byFile[file]!) '${s.line}  ${s.kind.label}',
            ],
            tokens: t,
          ),
      ],
    );
  }
}

class _Page extends StatelessWidget {
  const _Page({required this.stats, required this.children});

  final List<Widget> stats;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return SingleChildScrollView(
      padding: EdgeInsets.all(t.sp * 2),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Wrap(spacing: t.sp * 3, runSpacing: t.sp * 1.5, children: stats),
          SizedBox(height: t.sp * 2),
          ...children,
        ],
      ),
    );
  }
}

class _Group extends StatelessWidget {
  const _Group({
    required this.title,
    required this.count,
    required this.lines,
    required this.tokens,
    this.highlight = false,
  });

  final String title;
  final int count;
  final List<String> lines;
  final StormTokens tokens;
  final bool highlight;

  @override
  Widget build(BuildContext context) {
    final t = tokens;
    final style = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.codeSize,
      color: t.text,
    );
    return ColoredBox(
      color: highlight ? t.amberSoft : t.bg,
      child: ExpansionTile(
        enabled: lines.isNotEmpty,
        title: Text(title, style: style),
        trailing: CountBadge(
          count: count,
          color: highlight ? t.amber : t.accent,
        ),
        children: [
          Align(
            alignment: Alignment.centerLeft,
            child: Padding(
              padding: EdgeInsets.only(left: t.sp * 2, bottom: t.sp),
              child: SelectableText(
                lines.join('\n'),
                style: style.copyWith(color: t.text2),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
