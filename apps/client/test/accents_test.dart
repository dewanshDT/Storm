import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/state/app_state.dart';
import 'package:storm/state/vault_config.dart';
import 'package:storm/ui/accents.dart';
import 'package:storm/ui/tokens.dart';
import 'package:storm/ui/new_note_dialog.dart';
import 'package:storm/ui/shell/storm_scaffold.dart' show validateVaultPath;

/// Colours, fonts, and the new-note name.
///
/// The colour is written into the note as a *word* — `color: sage` — so the
/// tests that matter are about that word surviving a round trip and meaning
/// something in both themes, not about the exact hex.
final darkTokens = StormTokens.from(StormPreset.stormDark);
final lightTokens = StormTokens.from(StormPreset.stormLight);
StormTokens tokensFor(Brightness b) =>
    b == Brightness.dark ? darkTokens : lightTokens;

void main() {
  group('the palette', () {
    test('reads a colour back from the word in the file', () {
      expect(Accent.parse('sage'), Accent.sage);
      expect(Accent.parse('  SAGE  '), Accent.sage);
      expect(Accent.parse('Lavender'), Accent.lavender);
    });

    test('an unknown or absent colour is simply no colour', () {
      // A note written by hand, or by a later version with more colours, has
      // to open rather than fail.
      expect(Accent.parse(null), Accent.none);
      expect(Accent.parse(''), Accent.none);
      expect(Accent.parse('chartreuse'), Accent.none);
      expect(Accent.parse('#B7CDB0'), Accent.none);
    });

    test('every colour differs between light and dark', () {
      // A tint that works on white is a glare on black; sharing one value
      // would make half the palette unreadable in one of the two modes.
      for (final accent in Accent.values.where((a) => !a.isNone)) {
        expect(
          accent.tile(lightTokens),
          isNot(accent.tile(darkTokens)),
          reason: '${accent.name} uses one colour for both modes',
        );
      }
    });

    test('dark variants are dark and light variants are light', () {
      for (final accent in Accent.values.where((a) => !a.isNone)) {
        expect(
          accent.tile(lightTokens).computeLuminance(),
          greaterThan(0.5),
          reason: '${accent.name} is too dark for a light card',
        );
        expect(
          accent.tile(darkTokens).computeLuminance(),
          lessThan(0.35),
          reason: '${accent.name} is too bright for a dark card',
        );
      }
    });

    test('the page wash is quieter than the card fill', () {
      // The full tint behind a screen of prose fights the text.
      for (final mode in Brightness.values) {
        expect(
          Accent.sage.wash(tokensFor(mode)).a,
          lessThan(Accent.sage.tile(tokensFor(mode)).a),
        );
      }
    });

    test('names are stable, lower case and one word', () {
      // They are written into user files; churn here rewrites vaults.
      for (final accent in Accent.values) {
        expect(accent.name, matches(RegExp(r'^[a-z]+$')));
      }
    });
  });

  group('a vault carries its own colour', () {
    test('round-trips through the config note', () {
      const src = '---\nstorm.type.due: date\n---\n\n# Vault\n';
      final config = VaultConfig.parse(src);
      expect(config.accent, Accent.none);

      final coloured = VaultConfig.parse(config.withAccent(Accent.mint));
      expect(coloured.accent, Accent.mint);
      // And it did not disturb anything else.
      expect(coloured.types['due'], PropertyType.date);
    });

    test('clearing removes the key rather than writing none', () {
      final config = VaultConfig.parse(
        '---\nstorm.color: mint\n---\n\n# Vault\n',
      );
      final cleared = config.withAccent(Accent.none);
      expect(cleared, isNot(contains('storm.color')));
      expect(VaultConfig.parse(cleared).accent, Accent.none);
    });
  });

  group('a new note is named, not pathed', () {
    test('the extension is added, never typed', () {
      expect(noteFileName('Weekly review'), 'Weekly review.md');
      expect(noteFileName('Already.md'), 'Already.md');
      expect(noteFileName('SHOUTING.MD'), 'SHOUTING.MD');
    });

    test('the folder comes from where you are', () {
      expect(
        noteFileName('Plan', folder: 'Projects/Storm'),
        'Projects/Storm/Plan.md',
      );
    });

    test('separators become spaces instead of folders', () {
      // Typing `Q1/Plan` into a *name* field means the words. Creating a
      // directory nobody asked for would be a surprise.
      expect(noteFileName('Q1/Plan'), 'Q1 Plan.md');
      expect(noteFileName(r'Q1\Plan'), 'Q1 Plan.md');
    });

    test('a name cannot escape the vault or hide itself', () {
      // Both are rejected by the server's path rules, so they must be
      // impossible to produce here.
      expect(noteFileName('../escape'), 'escape.md');
      expect(noteFileName('.hidden'), 'hidden.md');
      expect(noteFileName('../../etc/passwd'), 'etc passwd.md');
    });

    test('an empty name still makes a note', () {
      expect(noteFileName(''), 'Untitled.md');
      expect(noteFileName('   '), 'Untitled.md');
      expect(noteFileName('...'), 'Untitled.md');
    });

    test('every generated name passes the vault path rules', () {
      for (final name in [
        'Weekly review',
        'Q1/Plan',
        '../escape',
        '.hidden',
        '',
        r'a\b/c',
      ]) {
        expect(
          validateVaultPath(noteFileName(name, folder: 'Notes')),
          isNull,
          reason: '"$name" produced a path the server would reject',
        );
      }
    });
  });

  group('the note font', () {
    test('defaults to the bundled serif', () {
      expect(const Settings().bodyFont, BodyFont.serif);
      expect(BodyFont.fromName(null), BodyFont.serif);
      expect(BodyFont.fromName('nonsense'), BodyFont.serif);
    });

    test('round-trips by name, since that is what is persisted', () {
      for (final font in BodyFont.values) {
        expect(BodyFont.fromName(font.name), font);
      }
    });

    test('sans means the platform default, not a bundled family', () {
      // Anything else would need shipping another megabyte of font.
      expect(BodyFont.sans.family, isNull);
      expect(BodyFont.serif.family, isNotNull);
    });
  });
}
