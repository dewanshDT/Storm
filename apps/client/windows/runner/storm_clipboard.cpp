#include "storm_clipboard.h"

#include <flutter/standard_method_codec.h>
#include <windows.h>

#include <shellapi.h>

#include <algorithm>
#include <cstdint>
#include <cwctype>
#include <fstream>
#include <iterator>
#include <optional>
#include <string>
#include <vector>

namespace {

using Bytes = std::vector<uint8_t>;

// Another program can hold the clipboard for a moment; it is not an error.
class OpenedClipboard {
 public:
  OpenedClipboard() {
    for (int i = 0; i < 5 && !open_; i++) {
      open_ = ::OpenClipboard(nullptr) != 0;
      if (!open_) ::Sleep(10);
    }
  }
  ~OpenedClipboard() {
    if (open_) ::CloseClipboard();
  }
  bool ok() const { return open_; }

 private:
  bool open_ = false;
};

std::optional<Bytes> global_bytes(HANDLE handle) {
  if (handle == nullptr) return std::nullopt;
  const SIZE_T size = ::GlobalSize(handle);
  const auto* data = static_cast<const uint8_t*>(::GlobalLock(handle));
  if (data == nullptr) return std::nullopt;
  Bytes bytes(data, data + size);
  ::GlobalUnlock(handle);
  return bytes;
}

// Kept byte for byte, as the macOS runner does; Dart re-encodes the rest.
bool is_image_file(const std::wstring& path) {
  std::wstring lower(path);
  std::transform(lower.begin(), lower.end(), lower.begin(),
                 [](wchar_t c) { return static_cast<wchar_t>(::towlower(c)); });
  static const wchar_t* const kept[] = {L".png", L".jpg",  L".jpeg",
                                        L".gif", L".webp", L".bmp"};
  for (const wchar_t* ext : kept) {
    const std::wstring e(ext);
    if (lower.size() > e.size() &&
        lower.compare(lower.size() - e.size(), e.size(), e) == 0) {
      return true;
    }
  }
  return false;
}

// A file copied in Explorer arrives as a path, not as image data.
std::optional<Bytes> copied_file() {
  auto drop = static_cast<HDROP>(::GetClipboardData(CF_HDROP));
  if (drop == nullptr || ::DragQueryFileW(drop, 0xFFFFFFFF, nullptr, 0) == 0) {
    return std::nullopt;
  }
  const UINT length = ::DragQueryFileW(drop, 0, nullptr, 0);
  std::wstring path(length + 1, L'\0');
  ::DragQueryFileW(drop, 0, path.data(), length + 1);
  path.resize(length);
  if (!is_image_file(path)) return std::nullopt;
  std::ifstream file(path, std::ios::binary);
  if (!file) return std::nullopt;
  return Bytes(std::istreambuf_iterator<char>(file), std::istreambuf_iterator<char>());
}

// CF_DIB is a BMP file without its 14-byte file header.
std::optional<Bytes> dib_as_bmp() {
  auto dib = global_bytes(::GetClipboardData(CF_DIB));
  if (!dib || dib->size() < sizeof(BITMAPINFOHEADER)) return std::nullopt;
  const auto* info = reinterpret_cast<const BITMAPINFOHEADER*>(dib->data());
  DWORD colors = info->biClrUsed;
  if (colors == 0 && info->biBitCount <= 8) colors = 1u << info->biBitCount;
  const DWORD masks =
      info->biSize == sizeof(BITMAPINFOHEADER) && info->biCompression == BI_BITFIELDS ? 12 : 0;
  BITMAPFILEHEADER header = {};
  header.bfType = 0x4D42;  // "BM"
  header.bfSize = static_cast<DWORD>(sizeof(header) + dib->size());
  header.bfOffBits = static_cast<DWORD>(sizeof(header) + info->biSize + masks +
                                        colors * sizeof(RGBQUAD));
  Bytes bmp(reinterpret_cast<const uint8_t*>(&header),
            reinterpret_cast<const uint8_t*>(&header) + sizeof(header));
  bmp.insert(bmp.end(), dib->begin(), dib->end());
  return bmp;
}

std::optional<Bytes> read_image() {
  OpenedClipboard clipboard;
  if (!clipboard.ok()) return std::nullopt;
  if (::IsClipboardFormatAvailable(CF_HDROP)) return copied_file();
  // Browsers, Office and the Snipping Tool also offer a lossless PNG.
  const UINT png = ::RegisterClipboardFormatW(L"PNG");
  if (png != 0 && ::IsClipboardFormatAvailable(png)) {
    if (auto bytes = global_bytes(::GetClipboardData(png))) return bytes;
  }
  if (::IsClipboardFormatAvailable(CF_DIB)) return dib_as_bmp();
  return std::nullopt;
}

}  // namespace

std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>>
AttachStormClipboard(flutter::BinaryMessenger* messenger) {
  auto channel = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      messenger, "storm/clipboard", &flutter::StandardMethodCodec::GetInstance());
  channel->SetMethodCallHandler(
      [](const flutter::MethodCall<flutter::EncodableValue>& call,
         std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result) {
        if (call.method_name() != "readImage") {
          result->NotImplemented();
          return;
        }
        auto image = read_image();
        if (image && !image->empty()) {
          result->Success(flutter::EncodableValue(std::move(*image)));
        } else {
          result->Success();
        }
      });
  return channel;
}
