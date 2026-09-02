// @ts-check
import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";

// Static marketing site. Hosted on Cloudflare Pages — never on the apt
// GitHub Pages root (https://dewanshdt.github.io/Storm/).
//
// `site` is not decoration: the sitemap, the canonical link and the absolute
// og:image in BaseLayout are all derived from it.
export default defineConfig({
  output: "static",
  site: "https://storm.dewansh.space",
  integrations: [sitemap()],
});
