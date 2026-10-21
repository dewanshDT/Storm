import 'package:flutter_test/flutter_test.dart';

import 'package:storm/state/replay_edit.dart';

/// Decision 128b: text typed while a save was in flight is replayed onto the
/// text the server answered with, never dropped.
void main() {
  test('an insert lands at the same place in the merged text', () {
    expect(
      replayEdit(
        sent: 'alpha\nbeta\n',
        edited: 'alpha\nbeta and more\n',
        server: 'TOP\nalpha\nbeta\n',
      ),
      'TOP\nalpha\nbeta and more\n',
    );
  });

  test('a deletion is replayed', () {
    expect(
      replayEdit(
        sent: 'one two three\nfour\n',
        edited: 'one three\nfour\n',
        server: 'one two three\nfour\nfive\n',
      ),
      'one three\nfour\nfive\n',
    );
  });

  test('a replacement is replayed', () {
    expect(
      replayEdit(
        sent: '# Title\n\nold words here\n',
        edited: '# Title\n\nnew words here\n',
        server: '# Title\n\nold words here\n\nfrom the phone\n',
      ),
      '# Title\n\nnew words here\n\nfrom the phone\n',
    );
  });

  test('an edit at the very end is replayed', () {
    expect(
      replayEdit(
        sent: 'intro\nlast line',
        edited: 'intro\nlast line!',
        server: 'INTRO\nintro\nlast line',
      ),
      'INTRO\nintro\nlast line!',
    );
  });

  test('an edit at the very start is replayed', () {
    expect(
      replayEdit(
        sent: 'body text\n',
        edited: 'Xbody text\n',
        server: 'body text\nmore\n',
      ),
      'Xbody text\nmore\n',
    );
  });

  test('nothing typed returns the server text unchanged', () {
    expect(replayEdit(sent: 'a\n', edited: 'a\n', server: 'a\nb\n'), 'a\nb\n');
  });

  test('an anchor that occurs twice is not guessed at', () {
    expect(
      replayEdit(sent: 'same\n', edited: 'same!\n', server: 'same\nsame\n'),
      isNull,
    );
  });

  test('an anchor the server text no longer holds is not guessed at', () {
    expect(
      replayEdit(
        sent: 'the line I edited\n',
        edited: 'the line I edited twice\n',
        server: 'completely different\n',
      ),
      isNull,
    );
  });

  test('an empty sent text has nothing to anchor to', () {
    expect(replayEdit(sent: '', edited: 'typed', server: 'other'), isNull);
  });
}
