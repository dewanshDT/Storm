import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/state/feedback_state.dart';

/// Decision 162c: the trail a report's diagnostics bundle draws from is
/// bounded and never carries a note title or path — only route/screen
/// shapes and palette command ids.
void main() {
  group('ActionTrail', () {
    test('bounded at 20, dropping the oldest', () {
      var trail = const ActionTrail();
      for (var i = 0; i < 25; i++) {
        trail = trail.recordCommand('cmd-$i');
      }
      expect(trail.actions, hasLength(20));
      // The first five fell off the front; the newest is last.
      expect(trail.actions.first, 'ran cmd-5');
      expect(trail.actions.last, 'ran cmd-24');
    });

    test('a repeated screen visit is a no-op, not a new entry', () {
      var trail = const ActionTrail();
      trail = trail.visitScreen('settings/health');
      final afterFirst = trail.actions.length;
      trail = trail.visitScreen('settings/health');
      expect(trail.actions.length, afterFirst);
      expect(trail.screen, 'settings/health');
    });

    test('a screen change is recorded and updates the current screen', () {
      var trail = const ActionTrail();
      trail = trail.visitScreen('notes/browse');
      trail = trail.visitScreen('agents/session');
      expect(trail.screen, 'agents/session');
      expect(trail.actions, ['opened notes/browse', 'opened agents/session']);
    });

    test('commands and screen visits share the same bounded ring', () {
      var trail = const ActionTrail();
      trail = trail.visitScreen('agents');
      trail = trail.recordCommand('new-session');
      expect(trail.actions, ['opened agents', 'ran new-session']);
    });
  });

  group('ActionTrailNotifier', () {
    test('reads back through the provider', () {
      final container = ProviderContainer();
      addTearDown(container.dispose);

      container.read(actionTrailProvider.notifier).visitScreen('notes');
      container.read(actionTrailProvider.notifier).recordCommand('search');

      final state = container.read(actionTrailProvider);
      expect(state.screen, 'notes');
      expect(state.actions, ['opened notes', 'ran search']);
    });
  });

  group('screenLabelOf', () {
    // Content-free: never a vault, note, session or crew id — only the
    // route's shape. Every case below carries an id the label must not.
    test(
      'note, browse, tags and trash collapse to their shape, dropping ids',
      () {
        expect(
          screenLabelOf(Uri.parse('/v/vault-123/note/note-456')),
          'notes/note',
        );
        expect(
          screenLabelOf(Uri.parse('/v/vault-123/browse/Projects/2026')),
          'notes/browse',
        );
        expect(screenLabelOf(Uri.parse('/v/vault-123/tags')), 'notes/tags');
        expect(screenLabelOf(Uri.parse('/v/vault-123/trash')), 'notes/trash');
      },
    );

    test('agent sessions and crews collapse to their shape', () {
      expect(
        screenLabelOf(Uri.parse('/agents/s/session-789')),
        'agents/session',
      );
      expect(screenLabelOf(Uri.parse('/agents/c/crew-abc')), 'agents/crew');
      expect(screenLabelOf(Uri.parse('/agents')), 'agents');
    });

    test('settings pages keep only the page id', () {
      expect(screenLabelOf(Uri.parse('/settings/health')), 'settings/health');
      expect(screenLabelOf(Uri.parse('/settings')), 'settings');
    });

    test('no label contains a uuid-shaped or otherwise long opaque id', () {
      final ids = [
        'vault-123',
        'note-456',
        'session-789',
        'crew-abc',
        'Projects/2026',
      ];
      for (final uri in [
        '/v/vault-123/note/note-456',
        '/v/vault-123/browse/Projects/2026',
        '/agents/s/session-789',
        '/agents/c/crew-abc',
      ]) {
        final label = screenLabelOf(Uri.parse(uri));
        for (final id in ids) {
          expect(label.contains(id), isFalse, reason: '$label leaked $id');
        }
      }
    });
  });
}
