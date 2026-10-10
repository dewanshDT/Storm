# The Storm Runtime Host for macOS (PLAN.md decision 83, AM41).
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
  url "https://github.com/dewanshDT/Storm.git", tag: "v0.6.0"
  license "MIT"
  head "https://github.com/dewanshDT/Storm.git", branch: "staging"

  depends_on "rust" => :build
  depends_on :macos

  def install
    # What release.yml does: Cargo.toml's version comes from the tag.
    inreplace "apps/runtime/Cargo.toml", /^version = .*$/, "version = \"#{version}\"" if build.stable?
    system "cargo", "install", *std_cargo_args(path: "apps/runtime")
  end

  def caveats
    <<~EOS
      storm-runtime runs as a LaunchDaemon under its own hidden account,
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
