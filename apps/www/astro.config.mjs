// @ts-check
import { defineConfig } from "astro/config";

// Static marketing site. Hosted on Cloudflare Pages — never on the apt
// GitHub Pages root (https://dewanshdt.github.io/Storm/).
export default defineConfig({
  output: "static",
  site: "https://storm.dewansh.space",
});
