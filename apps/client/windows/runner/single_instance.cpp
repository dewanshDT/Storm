#include "single_instance.h"

namespace {

constexpr wchar_t kMutexName[] = L"Local\\dev.storm.storm";
constexpr wchar_t kWindowClass[] = L"FLUTTER_RUNNER_WIN32_WINDOW";

}  // namespace

SingleInstance::SingleInstance() {
  mutex_ = ::CreateMutexW(nullptr, FALSE, kMutexName);
  // A mutex that cannot be created must not lock the person out of Storm.
  first_ = mutex_ == nullptr || ::GetLastError() != ERROR_ALREADY_EXISTS;
}

SingleInstance::~SingleInstance() {
  if (mutex_ != nullptr) {
    ::CloseHandle(mutex_);
  }
}

void FocusExistingWindow(const wchar_t* title) {
  HWND window = ::FindWindowW(kWindowClass, title);
  if (window == nullptr) {
    return;
  }
  if (::IsIconic(window)) {
    ::ShowWindow(window, SW_RESTORE);
  }
  ::SetForegroundWindow(window);
}
