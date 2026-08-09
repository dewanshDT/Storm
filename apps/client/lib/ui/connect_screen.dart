import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../api/storm_api.dart';
import '../state/app_state.dart';
import 'tokens.dart';
import 'widgets.dart';

/// First-run screen: point the app at a homelab server.
///
/// Verifies the connection before saving, so a typo surfaces here rather than
/// as an empty vault later.
class ConnectScreen extends ConsumerStatefulWidget {
  const ConnectScreen({super.key});

  @override
  ConsumerState<ConnectScreen> createState() => _ConnectScreenState();
}

class _ConnectScreenState extends ConsumerState<ConnectScreen> {
  final _url = TextEditingController(text: 'http://127.0.0.1:8484');
  final _token = TextEditingController();
  bool _testing = false;
  String? _error;

  @override
  void dispose() {
    _url.dispose();
    _token.dispose();
    super.dispose();
  }

  Future<void> _connect() async {
    setState(() {
      _testing = true;
      _error = null;
    });

    final url = normalizeUrl(_url.text);
    final api = StormApi(baseUrl: url, token: _token.text.trim());
    try {
      await api.checkConnection();
      if (!mounted) return;

      final current = ref.read(settingsProvider).value ?? const Settings();
      await ref
          .read(settingsProvider.notifier)
          .save(current.copyWith(baseUrl: url, token: _token.text.trim()));
    } on StormApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.isUnauthorized
            ? 'The server rejected that token.'
            : 'Server error: ${e.message}';
      });
    } catch (e) {
      if (!mounted) return;
      setState(
        () => _error = "Couldn't reach the server. Is it running?\n\n$e",
      );
    } finally {
      api.dispose();
      if (mounted) setState(() => _testing = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: t.sp * 52),
          child: Padding(
            padding: EdgeInsets.all(t.cardPad),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                // The mark, not the word: the design is explicit that a text
                // wordmark is never substituted for it.
                const Center(child: BrandMark(size: 44, withWordmark: true)),
                SizedBox(height: t.sp * 2),
                Text(
                  'Connect to your homelab vault',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 3.5),
                StormInput(
                  controller: _url,
                  autofocus: true,
                  labelText: 'Server address',
                  hintText: '192.168.1.20:8484',
                  keyboardType: TextInputType.url,
                  autocorrect: false,
                ),
                SizedBox(height: t.sp * 1.75),
                StormInput(
                  controller: _token,
                  labelText: 'Access token',
                  obscureText: true,
                  autocorrect: false,
                  onSubmitted: (_) => _connect(),
                ),
                if (_error != null) ...[
                  SizedBox(height: t.sp * 2),
                  Container(
                    padding: EdgeInsets.all(t.sp * 1.5),
                    decoration: BoxDecoration(
                      color: t.surface,
                      borderRadius: BorderRadius.circular(t.rControl),
                      border: Border.all(color: t.danger, width: t.bw),
                    ),
                    child: Text(
                      _error!,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        color: t.danger,
                      ),
                    ),
                  ),
                ],
                SizedBox(height: t.sp * 2.5),
                FilledButton(
                  onPressed: _testing ? null : _connect,
                  child: _testing
                      ? const SizedBox(
                          height: 18,
                          width: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Text('Connect'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
