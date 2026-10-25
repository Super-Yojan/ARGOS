// @ts-check
import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";
import pitchTheme from "./src/styles/shiki-pitch.json" with { type: "json" };
import { remarkCallout } from "./src/plugins/remark-callout.mjs";

// A GitHub project Page under super-yojan.dev, styled like the user site.
export default defineConfig({
  site: "https://super-yojan.dev",
  base: "/ARGOS",
  integrations: [sitemap()],
  markdown: {
    remarkPlugins: [remarkCallout],
    // The blog's pitch theme: greys plus the teal ramp, one row per line.
    shikiConfig: {
      theme: pitchTheme,
      wrap: false,
    },
  },
});
