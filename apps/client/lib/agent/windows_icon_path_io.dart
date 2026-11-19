import 'dart:io';

/// The on-disk `.ico` beside `storm.exe` (review item 9, decision 161):
/// Windows' `IconUri` registry value — what
/// `WindowsInitializationSettings.iconPath` ultimately writes
/// (`flutter_local_notifications_windows`'s `plugin.cpp`,
/// `UpdateRegistry`) — wants a real file on disk, never the copy the build
/// embeds into the exe's own resources through `Runner.rc`'s `IDI_APP_ICON`
/// (compiled in, not a loose file). `tool/package_windows.sh` copies
/// `windows/runner/resources/app_icon.ico` next to the exe at package time
/// for exactly this; a dev build run straight out of
/// `build/windows/x64/runner/{Debug,Release}` without packaging has no such
/// file yet, so the registry value points nowhere until packaged — the
/// Windows toast still shows, just with the generic icon, same as today.
String windowsIconPath() =>
    '${File(Platform.resolvedExecutable).parent.path}'
    '${Platform.pathSeparator}app_icon.ico';
