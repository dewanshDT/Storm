/// The web build has no executable to sit beside, and Windows toast
/// branding never applies to it — [RealOsNotifications] only ever runs on
/// desktop. Never actually read; this exists only so the facade compiles
/// for the web target too.
String windowsIconPath() => '';
