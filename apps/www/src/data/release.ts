/**
 * Public release + install URLs for www.
 * Bump `tag` when cutting a release; asset names follow release.yml.
 *
 * Apt install lives on GitHub Pages (decision 49) — not on the marketing host.
 */
export const release = {
  tag: "v0.2.3",
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
    return `curl -fsSL ${this.installScriptUrl} | sudo sh`;
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
      note: `Default port ${this.webPort} after storm-server up.`,
      file,
      url: `${this.assetBase}/${file}`,
      detect: "web" as const,
      action: "open" as const,
      cta: "Open →",
      aria: "How to open the Storm web client on your server",
    };
  },
} as const;
