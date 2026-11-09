# The Storm Runtime Host for macOS (decision 83, AM41).
#
# This repository is the tap:
#   brew tap dewanshdt/storm https://github.com/dewanshDT/Storm
#   brew install storm-runtime
#
# Built from the release tag, so the tag stays the only version source
# (decision 46): `tag` is bumped in the release-prep PR with
# apps/www/src/data/release.ts, and `make formula-check` fails when it is older.
class StormRuntime < Formula
  desc "Storm Runtime Host: runs agent sessions for a Storm Server"
  homepage "https://github.com/dewanshDT/Storm"
  url "https://github.com/dewanshDT/Storm.git", tag: "v0.14.0"
  license "MIT"
  head "https://github.com/dewanshDT/Storm.git", branch: "staging"

  depends_on "rust" => :build
  depends_on :macos

  def install
    # What release.yml does: Cargo.toml's version comes from the tag.
    inreplace "apps/runtime/Cargo.toml", /^version = .*$/, "version = \"#{version}\"" if build.stable?
    system "cargo", "install", *std_cargo_args(path: "apps/runtime")
  end

  # `brew services start storm-runtime`: the host as you, a LaunchAgent with
  # your own state (decision 151 E2), as `storm-runtime install --user` does.
  # Never with sudo, which would run it as root on the service's state.
  service do
    run [opt_bin/"storm-runtime", "serve"]
    run_type :immediate
    # Not after a clean or refused exit: unenrolled and revoked hosts stay down.
    keep_alive crashed: true
    process_type :standard
    environment_variables LANG: "en_US.UTF-8"
    log_path var/"log/storm-runtime.log"
    error_log_path var/"log/storm-runtime.log"
  end

  def caveats
    <<~EOS
      To run the host as yourself, in your own session (your logins, your
      Keychain, your folders; it stops when you log out), enroll it, then
      start it with brew:
        #{opt_bin}/storm-runtime enroll
        brew services start storm-runtime
      Never `sudo brew services`. `storm-runtime install --user` does the same
      without brew; use one or the other.

      Or run it as a LaunchDaemon under its own hidden account,
      _stormruntime, never as you. Install the service (copies this binary to
      /Library/StormRuntime/bin, creates the account and the plist):
        sudo #{opt_bin}/storm-runtime install

      Then, in the Storm app: Settings > Agents > Hosts > Enroll a host, and
      paste the string at this prompt (never as an argument):
        cd / && sudo -u _stormruntime /Library/StormRuntime/bin/storm-runtime enroll

      launchd starts the host by itself once it is enrolled. Workspaces are the
      directories under /Library/StormRuntime/workspaces.

      Upgrading needs the install step again, which replaces the daemon's copy:
        brew upgrade storm-runtime && sudo #{opt_bin}/storm-runtime install

      Remove the service (add --purge to remove its state and account too;
      workspaces are always kept):
        sudo /Library/StormRuntime/bin/storm-runtime uninstall
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/storm-runtime --version")
    assert_match "enroll", shell_output("#{bin}/storm-runtime --help")
  end
end
