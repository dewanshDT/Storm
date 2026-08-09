/// Empty, loading, offline and conflict.
///
/// The rule that shapes all of them: **offline is a condition, never an
/// error** — it says what you are looking at and what happens next.
library;

import 'package:flutter/material.dart';

import '../api/models.dart';

import 'widgets.dart';
import 'tokens.dart';

/// Turns any failure into something the user can read.
///
/// A `StormApiException` is the server saying no, and its wording is the best
/// available. Anything else is the server not answering at all — which used to
/// escape as an unhandled exception and put a red screen in front of a folder
/// that had, in some cases, actually been created.
String describeFailure(Object e) => e is StormApiException
    ? e.message
    : 'Could not reach the server. Try again when it is back.';

/// Nothing here yet, and what to do about it.
class EmptyState extends StatelessWidget {
  const EmptyState({
    super.key,
    required this.icon,
    required this.title,
    this.detail,
    this.action,
    this.onAction,
  });

  final IconData icon;
  final String title;
  final String? detail;
  final String? action;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: t.cardPad,
        vertical: t.sectionRhythm * 0.7,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: t.headingSize * 1.5, color: t.text3),
          SizedBox(height: t.sp * 1.5),
          Text(
            title,
            textAlign: TextAlign.center,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.bodySize,
              fontWeight: FontWeight.w600,
              color: t.text,
            ),
          ),
          if (detail != null) ...[
            SizedBox(height: t.sp * 0.75),
            Text(
              detail!,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.text3,
              ),
            ),
          ],
          if (action != null && onAction != null) ...[
            SizedBox(height: t.sp * 2),
            FilledButton(onPressed: onAction, child: Text(action!)),
          ],
        ],
      ),
    );
  }
}

/// Grey bars where rows are about to be.
///
/// A skeleton rather than a spinner: these lists come from cache and arrive
/// almost immediately, and a spinner in that window reads as "something is
/// wrong".
class SkeletonRows extends StatelessWidget {
  const SkeletonRows({super.key, this.rows = 4});

  final int rows;

  static const _widths = [0.62, 0.44, 0.71, 0.38, 0.55, 0.48];

  // Deliberately still. A repeating shimmer never lets `pumpAndSettle`
  // return, so every widget test that renders a loading state hangs — and
  // these lists come from cache and are gone in a frame or two anyway.
  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.symmetric(vertical: t.sp),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (var i = 0; i < rows; i++)
            Padding(
              padding: EdgeInsets.only(bottom: t.sp * 2),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  _Bar(
                    widthFactor: _widths[i % _widths.length],
                    height: t.sp * 1.625,
                  ),
                  SizedBox(height: t.sp * 0.75),
                  _Bar(
                    widthFactor: _widths[(i + 2) % _widths.length] * 0.6,
                    height: t.sp * 1.125,
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

class _Bar extends StatelessWidget {
  const _Bar({required this.widthFactor, required this.height});

  final double widthFactor;
  final double height;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return FractionallySizedBox(
      alignment: Alignment.centerLeft,
      widthFactor: widthFactor,
      child: Container(
        height: height,
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl * 0.4),
        ),
      ),
    );
  }
}

/// The server cannot be reached, and that is all it means.
///
/// Grey, never red: nothing is broken and nothing is lost.
class OfflineNotice extends StatelessWidget {
  const OfflineNotice({super.key, this.queued = 0, this.onRetry});

  final int queued;
  final VoidCallback? onRetry;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final queuedText = switch (queued) {
      0 => '',
      1 => ' 1 edit is queued and will send when the server is reachable.',
      _ =>
        ' $queued edits are queued and will send when the server is '
            'reachable.',
    };

    return Container(
      margin: EdgeInsets.symmetric(horizontal: t.cardPad, vertical: t.sp),
      padding: EdgeInsets.all(t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface,
        borderRadius: BorderRadius.circular(t.rCard),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: EdgeInsets.only(top: t.sp * 0.5),
            child: const StatusDot(status: DotStatus.offline),
          ),
          SizedBox(width: t.sp * 1.25),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Offline',
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.bodySize,
                    fontWeight: FontWeight.w600,
                    color: t.text,
                  ),
                ),
                SizedBox(height: t.sp * 0.25),
                Text(
                  'Showing your cached copy.$queuedText',
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
              ],
            ),
          ),
          if (onRetry != null) ...[
            SizedBox(width: t.sp),
            TextButton(onPressed: onRetry, child: const Text('Retry now')),
          ],
        ],
      ),
    );
  }
}

/// Two versions of this note exist, and both are in the text.
///
/// Explains the mechanism rather than naming the problem, because resolving it
/// means editing the note by hand.
class ConflictCard extends StatelessWidget {
  const ConflictCard({super.key, this.onDismiss});

  final VoidCallback? onDismiss;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      margin: EdgeInsets.symmetric(horizontal: t.cardPad, vertical: t.sp),
      padding: EdgeInsets.all(t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface,
        borderRadius: BorderRadius.circular(t.rCard),
        border: Border.all(color: t.danger, width: t.bw),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(Icons.merge_type, size: t.bodySize, color: t.danger),
              SizedBox(width: t.sp * 0.75),
              Expanded(
                child: Text(
                  'Conflict in this note',
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.bodySize,
                    fontWeight: FontWeight.w600,
                    color: t.danger,
                  ),
                ),
              ),
              if (onDismiss != null)
                InkWell(
                  onTap: onDismiss,
                  child: Icon(Icons.close, size: t.bodySize, color: t.text3),
                ),
            ],
          ),
          SizedBox(height: t.sp * 0.75),
          Text(
            'Another device saved a different version. Both are kept in the '
            'text below — delete the lines you don’t want.',
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.text2,
            ),
          ),
          SizedBox(height: t.sp),
          Container(
            width: double.infinity,
            padding: EdgeInsets.symmetric(
              horizontal: t.sp,
              vertical: t.sp * 0.75,
            ),
            decoration: BoxDecoration(
              color: t.surface2,
              borderRadius: BorderRadius.circular(t.rControl * 0.6),
            ),
            child: Text(
              '<<<<<<< yours',
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize,
                color: t.text3,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
