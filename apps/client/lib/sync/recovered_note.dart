/// An unsynced edit to a note that was deleted elsewhere, kept as a new note.
class RecoveredNote {
  const RecoveredNote({
    required this.oldId,
    required this.newId,
    required this.path,
  });

  final String oldId;
  final String newId;
  final String path;
}
