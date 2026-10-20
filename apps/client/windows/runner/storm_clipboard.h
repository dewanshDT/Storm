#ifndef RUNNER_STORM_CLIPBOARD_H_
#define RUNNER_STORM_CLIPBOARD_H_

#include <flutter/binary_messenger.h>
#include <flutter/encodable_value.h>
#include <flutter/method_channel.h>

#include <memory>

// The storm/clipboard channel (D15 AM45, decision 125c): readImage answers
// with the clipboard's image as bytes, or null. Keep the returned channel for
// as long as the engine runs.
std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>>
AttachStormClipboard(flutter::BinaryMessenger* messenger);

#endif  // RUNNER_STORM_CLIPBOARD_H_
