import { defineCollection } from "astro:content";
import { glob } from "astro/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

// The docs are written in the repo's docs/site/, next to the code they
// describe, so a feature and its page land in one PR. Every id is prefixed
// with `docs/` so the pages serve at /docs/… beside the marketing routes.
const toId = (entry: string) => {
  const parts = entry
    .replace(/\.mdx?$/, "")
    .split("/")
    .map((p) => p.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, ""));
  if (parts[parts.length - 1] === "index") parts.pop();
  return ["docs", ...parts].join("/");
};

export const collections = {
  docs: defineCollection({
    loader: glob({
      pattern: "**/[^_]*.{md,mdx}",
      base: "../../docs/site",
      generateId: ({ entry }) => toId(entry),
    }),
    schema: docsSchema(),
  }),
};
