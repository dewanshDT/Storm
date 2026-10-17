#ifndef RUNNER_SINGLE_INSTANCE_H_
#define RUNNER_SINGLE_INSTANCE_H_

#include <windows.h>

inline constexpr wchar_t kWindowTitle[] = L"Storm";

// One Storm per signed-in user: a named mutex in the session's Local
// namespace, held for the life of the process.
class SingleInstance {
 public:
  SingleInstance();
  ~SingleInstance();

  SingleInstance(const SingleInstance&) = delete;
  SingleInstance& operator=(const SingleInstance&) = delete;

  bool IsFirst() const { return first_; }

 private:
  HANDLE mutex_ = nullptr;
  bool first_ = true;
};

// Restores and raises the running instance's window, if it can be found.
void FocusExistingWindow(const wchar_t* title);

#endif  // RUNNER_SINGLE_INSTANCE_H_
