/**
 * Public release + install URLs for www.
 * Bump `tag` when cutting a release; asset names follow release.yml.
 *
 * Apt install lives on GitHub Pages (decision 49) — not on the marketing host.
 */
export const release = {
  tag: "v0.11.0",
  get version() {
    return this.tag.replace(/^v/, "");
  },
  get notesUrl() {
    return `https://github.com/dewanshDT/Storm/releases/tag/${this.tag}`;
  },
  allUrl: "https://github.com/dewanshDT/Storm/releases",
  latestUrl: "https://github.com/dewanshDT/Storm/releases/latest",
  /** Default listen port from deploy/storm.env.example */
  webPort: 8484,
  /** Apt repository root — never the marketing site. */
  aptRoot: "https://dewanshdt.github.io/Storm/",
  get installScriptUrl() {
    return `${this.aptRoot}install.sh`;
  },
  get installCommand() {
    // The interactive installer (decision 84) runs as the user and asks for
    // sudo only for the steps that need it.
    return `curl -fsSL ${this.installScriptUrl} | sh`;
  },
  /** Windows counterpart of install.sh (decision 99), from the same root. */
  get windowsInstallCommand() {
    return `irm ${this.aptRoot}install.ps1 | iex`;
  },
  /** Re-running the installer upgrades whatever it installed. */
  get installerUpgradeCommand() {
    return `curl -fsSL ${this.installScriptUrl} | sh -s -- upgrade`;
  },
  /** Apt already registered — refresh binary + bundled web client. */
  get upgradeCommand() {
    return `sudo apt update
sudo apt install --only-upgrade storm-server
sudo systemctl restart storm-server
sudo storm-server status`;
  },
  get assetBase() {
    return `https://github.com/dewanshDT/Storm/releases/download/${this.tag}`;
  },
  get checksumsUrl() {
    return `${this.assetBase}/checksums.txt`;
  },
  get macos() {
    const file = `Storm-${this.version}-macos-arm64.zip`;
    return {
      id: "macos" as const,
      platform: "macOS",
      meta: "Apple Silicon · arm64",
      blurb: "Native desktop client.",
      note: "Ad-hoc signed. macOS may ask you to allow it on first launch.",
      file,
      url: `${this.assetBase}/${file}`,
      detect: "mac" as const,
      action: "download" as const,
      cta: "Download →",
      aria: `Download Storm for macOS (${this.tag})`,
    };
  },
  /** Named without a version, like every Windows asset (decision 125d). */
  get windows() {
    const file = "storm-app-windows-x86_64.zip";
    return {
      id: "windows" as const,
      platform: "Windows",
      meta: "Windows 10/11 · x64",
      blurb: "Native desktop client. Unzip anywhere and run storm.exe.",
      note: "Not code-signed: SmartScreen may warn. Or install it with the PowerShell installer.",
      file,
      url: `${this.assetBase}/${file}`,
      detect: "windows" as const,
      action: "download" as const,
      cta: "Download →",
      aria: `Download Storm for Windows (${this.tag})`,
    };
  },
  /** The .deb; the tarball is for other distributions (decision 125f). */
  get linux() {
    const file = `storm-app_${this.version}-1_amd64.deb`;
    return {
      id: "linux" as const,
      platform: "Linux",
      meta: "Debian · Ubuntu · x86_64",
      blurb: "Native desktop client.",
      note: `Other distributions: storm-app-${this.version}-x86_64-linux.tar.gz, or the installer's --app.`,
      file,
      url: `${this.assetBase}/${file}`,
      detect: "linux" as const,
      action: "download" as const,
      cta: "Download →",
      aria: `Download Storm for Linux (${this.tag})`,
    };
  },
  get android() {
    const file = `storm-${this.version}.apk`;
    return {
      id: "android" as const,
      platform: "Android",
      meta: "APK",
      blurb: "Android client for your Storm server.",
      note: "Direct APK. Android may ask you to allow installation from this source.",
      file,
      url: `${this.assetBase}/${file}`,
      detect: "android" as const,
      action: "download" as const,
      cta: "Download →",
      aria: `Download Storm APK for Android (${this.tag})`,
    };
  },
  get web() {
    const file = `storm-web-${this.version}.zip`;
    return {
      id: "web" as const,
      platform: "Web",
      meta: "Served by Storm Server",
      blurb: "Bundled with the server. Open your Storm server URL in a browser.",
      note: `Default port ${this.webPort} after storm-server up. On first open it pairs itself with the server and asks you to set your Storm's password.`,
      file,
      url: `${this.assetBase}/${file}`,
      detect: "web" as const,
      action: "open" as const,
      cta: "Open →",
      aria: "How to open the Storm web client on your server",
    };
  },
} as const;
