/// The one place an error snackbar's "Report this" action lives (decision
/// 162a) — every call site that shows a genuine failure, rather than a
/// success or an informational notice, goes through here instead of raising
/// its own `SnackBar`.
library;

import 'package:flutter/material.dart';

import '../api/feedback_models.dart' show FeedbackErrorContext;
import '../state/app_state.dart' show rootMessengerKey, rootNavigatorKey;
import 'report_sheet.dart' show showReportProblemSheet;

/// Shows [message] on the app's one [ScaffoldMessenger]. When
/// [reportContext] is given, the snackbar carries a "Report this" action
/// that opens the report sheet with the failure already filled in.
///
/// Takes no `BuildContext` on purpose — some of its call sites (a sync
/// notice, say) run inside a provider with none of its own, so this reaches
/// the navigator through [rootNavigatorKey] instead.
void showErrorSnack(String message, {FeedbackErrorContext? reportContext}) {
  rootMessengerKey.currentState?.showSnackBar(
    SnackBar(
      content: Text(message),
      action: reportContext == null
          ? null
          : SnackBarAction(
              label: 'Report this',
              onPressed: () {
                final context = rootNavigatorKey.currentContext;
                if (context != null) {
                  showReportProblemSheet(context, error: reportContext);
                }
              },
            ),
    ),
  );
}
