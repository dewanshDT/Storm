/// How a session's status is drawn (handoff §3.2): accent for live work, a
/// ring while starting, danger for failure, text3 for everything else.
library;

import 'package:flutter/material.dart';

import 'tokens.dart';

class SessionStatusLook {
  const SessionStatusLook({
    required this.label,
    required this.color,
    required this.live,
    this.ring = false,
  });

  final String label;
  final Color color;

  /// In the Running group (input may still be refused while unknown).
  final bool live;
  final bool ring;
}

SessionStatusLook sessionStatusLook(
  String status,
  StormTokens t,
) => switch (status) {
  'creating' || 'starting' => SessionStatusLook(
    label: 'Starting',
    color: t.accent,
    live: true,
    ring: true,
  ),
  'running' => SessionStatusLook(label: 'Running', color: t.accent, live: true),
  'unknown' => SessionStatusLook(label: 'Unknown', color: t.text3, live: true),
  'completed' => SessionStatusLook(
    label: 'Completed',
    color: t.text3,
    live: false,
  ),
  'stopped' => SessionStatusLook(label: 'Stopped', color: t.text3, live: false),
  'failed' => SessionStatusLook(label: 'Failed', color: t.danger, live: false),
  _ => SessionStatusLook(label: status, color: t.text3, live: false),
};

/// 8px, or a 2px ring while starting.
class SessionStatusDot extends StatelessWidget {
  const SessionStatusDot({super.key, required this.status, this.size});

  final String status;
  final double? size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final look = sessionStatusLook(status, t);
    final side = size ?? t.sp;
    return Container(
      width: side,
      height: side,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: look.ring ? null : look.color,
        border: look.ring
            ? Border.all(color: look.color, width: t.bw * 2)
            : null,
      ),
    );
  }
}

/// Mono, outlined, fully rounded, a 7px dot and the label in the status
/// colour. [StatusPill] is the same shape for hosts and integrations.
class SessionStatusChip extends StatelessWidget {
  const SessionStatusChip({super.key, required this.status});

  final String status;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final look = sessionStatusLook(status, t);
    return StatusPill(
      label: look.label,
      color: look.color,
      dot: SessionStatusDot(status: status, size: t.sp * 0.875),
    );
  }
}

class StatusPill extends StatelessWidget {
  const StatusPill({
    super.key,
    required this.label,
    required this.color,
    this.dot,
  });

  final String label;
  final Color color;
  final Widget? dot;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 1.125,
        vertical: t.sp * 0.25,
      ),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(999),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (dot != null) ...[dot!, SizedBox(width: t.sp * 0.75)],
          Text(
            label,
            style: TextStyle(
              fontFamily: StormTokens.monoFamily,
              fontSize: t.labelSize,
              color: color,
              height: 1.3,
            ),
          ),
        ],
      ),
    );
  }
}
