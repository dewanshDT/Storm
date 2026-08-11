/**
 * Current public release — single source for www download links.
 * Bump `tag` when cutting a release; asset names follow release.yml.
 */
export const release = {
  tag: "v0.2.2",
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
      file,
      url: `${this.assetBase}/${file}`,
      detect: "mac" as const,
    };
  },
  get android() {
    const file = `storm-${this.version}.apk`;
    return {
      id: "android" as const,
      platform: "Android",
      meta: `APK · ${this.tag}`,
      file,
      url: `${this.assetBase}/${file}`,
      detect: "android" as const,
    };
  },
  get web() {
    const file = `storm-web-${this.version}.zip`;
    return {
      id: "web" as const,
      platform: "Web",
      meta: "Served by Storm Server",
      file,
      url: `${this.assetBase}/${file}`,
      detect: "web" as const,
    };
  },
} as const;
