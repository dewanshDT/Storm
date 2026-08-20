import 'package:flutter/material.dart';

import 'widgets.dart';

/// Shown while the app works out which screen it belongs on.
///
/// Without it the router has only wrong answers during startup. `null` means
/// "stay put", which on a cold start is the dashboard — so a fresh browser
/// rendered the vault shell, then the QR pairing screen while its bootstrap
/// was in flight, then the login screen: two screens nobody should have seen,
/// each for long enough to register and not long enough to read.
///
/// A brand mark and nothing else. This is a state the app passes through, not
/// one it should look busy in.
class StartingScreen extends StatelessWidget {
  const StartingScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return const Scaffold(
      body: Center(child: BrandMark(size: 44, withWordmark: true)),
    );
  }
}
