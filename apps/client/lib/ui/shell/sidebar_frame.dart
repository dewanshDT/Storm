import 'package:flutter/material.dart';

import '../breakpoints.dart';
import '../tokens.dart';

/// The one sidebar the Notes, Agents and Settings shells share (handoff §8):
/// `surface`, a right hairline, the scaled sidebar width, and an optional
/// footer above its own hairline.
class SidebarFrame extends StatelessWidget {
  const SidebarFrame({super.key, required this.body, this.footer});

  final Widget body;
  final Widget? footer;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    // Material, not a coloured box: rows ink onto the nearest Material.
    return Material(
      color: t.surface,
      shape: Border(
        right: BorderSide(color: t.border, width: t.bw),
      ),
      child: SizedBox(
        width: context.sidebarWidth,
        child: SafeArea(
          right: false,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(child: body),
              if (footer != null) ...[
                Container(height: t.bw, color: t.border),
                footer!,
              ],
            ],
          ),
        ),
      ),
    );
  }
}
