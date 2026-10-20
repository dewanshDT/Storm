#ifndef RUNNER_STORM_CLIPBOARD_H_
#define RUNNER_STORM_CLIPBOARD_H_

#include <flutter_linux/flutter_linux.h>

// The storm/clipboard channel (D15 AM45, decision 125c): readImage answers
// with the clipboard's image as bytes, or null. The channel lives as long as
// the view it is attached to.
void storm_clipboard_attach(FlView* view);

#endif  // RUNNER_STORM_CLIPBOARD_H_
