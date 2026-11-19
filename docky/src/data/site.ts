/** Links point at the public repository. `download` always serves the newest
 * published release (the release workflow attaches a stable Docky-Setup.exe). */
export const site = {
  name: "Docky",
  tagline: "The macOS Dock, made for Windows.",
  /** Kept concise for search snippets while naming the supported Windows versions. */
  description:
    "Free, open-source macOS-style dock for Windows 10 and 11, with the Genie effect, optional live window previews and an AI usage menu bar.",
  /** Canonical origin. Must match `site` in astro.config.mjs and the sitemap. */
  url: "https://latief.id/docky",
  /** Path to the social card in `public/`. */
  ogImage: "/og-image.png",
  ogImageWidth: 1200,
  ogImageHeight: 630,
  locale: "en_US",
  repository: "https://github.com/latiefahmad/Docky",
  releases: "https://github.com/latiefahmad/Docky/releases",
  download: "https://github.com/latiefahmad/Docky/releases/latest/download/Docky-Setup.exe",
  issues: "https://github.com/latiefahmad/Docky/issues",
  version: "1.0.1",
  license: "MIT",
} as const;
