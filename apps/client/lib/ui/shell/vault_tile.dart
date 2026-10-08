import 'package:flutter/material.dart';

import '../accents.dart';
import '../tokens.dart';
import '../widgets.dart' show kTileInk;

/// A vault's accent tile with its initial in mono (handoff §7.1): 28 in
/// lists, 30 in headers.
class VaultTile extends StatelessWidget {
  const VaultTile({
    super.key,
    required this.name,
    required this.accent,
    required this.size,
  });

  final String name;
  final Accent accent;
  final double size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final initial = name.trim().isEmpty ? 'S' : name.trim()[0].toUpperCase();
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: accent.tile(t),
        borderRadius: BorderRadius.circular(t.rControl * 0.8),
      ),
      child: Text(
        initial,
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: size * 0.46,
          fontWeight: FontWeight.w500,
          color: accent.isNone ? t.text2 : kTileInk,
        ),
      ),
    );
  }
}
