import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

/// The redirect URI Android and macOS register for integration sign-ins
/// (spec §10.4, decision 81l).
const appRedirect = 'storm://oauth/callback';

/// `storm://oauth` redirects as the platform delivers them.
///
/// The native side (`MainActivity.kt`, `AppDelegate.swift`) buffers every
/// redirect until Dart asks for them with `takeLinks`, then forwards new ones
/// as `link` calls. So a redirect that launched the app is not lost to the
/// race between the platform and Dart's first frame.
///
/// A redirect goes to the sign-in waiting for its `state`. **One nobody is
/// waiting for is kept as an orphan**, not dropped: the app was killed while
/// the browser was open, or the platform started a fresh instance for the
/// link. The sign-in itself lives on the server (the flow, PKCE and all), so
/// relaying its `{state, code}` from whichever instance received it still
/// finishes it. The Integrations screen does that.
class OAuthLinks {
  OAuthLinks({MethodChannel? channel})
    : _channel = channel ?? const MethodChannel('storm/links');

  /// Links delivered only through [deliver]: no platform channel, so no
  /// Flutter binding needed. For tests of the code that consumes links.
  OAuthLinks.detached() : _channel = null;

  /// The app's one instance, bound to the platform channel.
  static final instance = OAuthLinks();

  final MethodChannel? _channel;
  final _waiting = <String, Completer<Map<String, String>>>{};

  /// Redirects no sign-in in this process was waiting for, oldest first.
  final orphans = ValueNotifier<List<Map<String, String>>>(const []);

  Future<void>? _started;

  /// Starts listening, once, and takes whatever the platform buffered. A
  /// platform without the scheme (Linux, Windows, tests) has no handler on
  /// the other side, which is not an error.
  Future<void> start() => _started ??= _start();

  Future<void> _start() async {
    final channel = _channel;
    if (channel == null) return;
    channel.setMethodCallHandler((call) async {
      if (call.method == 'link' && call.arguments is String) {
        deliver(Uri.parse(call.arguments as String));
      }
    });
    try {
      final buffered =
          await channel.invokeListMethod<String>('takeLinks') ?? const [];
      for (final link in buffered) {
        deliver(Uri.parse(link));
      }
    } on MissingPluginException {
      // No native side: nothing was buffered.
    }
  }

  /// One redirect. Anything but `storm://oauth…` with a `state` is ignored:
  /// the scheme is public, so any app or page can open it.
  void deliver(Uri uri) {
    if (uri.scheme != 'storm' || uri.host != 'oauth') return;
    final params = uri.queryParameters;
    final state = params['state'];
    if (state == null || state.isEmpty) return;
    final waiter = _waiting.remove(state);
    if (waiter != null) {
      waiter.complete(params);
      return;
    }
    orphans.value = [...orphans.value, params];
  }

  /// The redirect carrying `state`, when it comes.
  Future<Map<String, String>> waitFor(String state) {
    final waiter = Completer<Map<String, String>>();
    _waiting[state] = waiter;
    return waiter.future;
  }

  /// This sign-in is over (finished, failed or given up): a redirect for it
  /// that arrives later is an orphan, relayed and refused by the server as
  /// spent, rather than completing nothing.
  void cancel(String state) => _waiting.remove(state);

  /// Every orphan so far, now the caller's to relay.
  List<Map<String, String>> takeOrphans() {
    final taken = orphans.value;
    orphans.value = const [];
    return taken;
  }
}

/// The [OAuthLinks] the app uses. Tests override it.
final oauthLinksProvider = Provider<OAuthLinks>((ref) => OAuthLinks.instance);
