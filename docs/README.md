# docs

This directory is two things:

- **The documentation site** — a [Fumadocs](https://fumadocs.dev) app (Next.js
  static export), deployed to GitHub Pages at https://t41372.github.io/skit/ by
  `.github/workflows/docs.yml` on pushes to main.
- **Repo doc assets** — `assets/` (media the READMEs hotlink via
  raw.githubusercontent; the demo `.mp4`s are deliberately untracked) and
  `design/` (internal design notes). Neither is published to the site.

## Editing content

Pages are the MDX files in `content/docs/`. English is the source locale. Each English page has
`.<locale>.mdx` siblings for Simplified Chinese and Traditional Chinese. The sidebar files are
`meta.json`, `meta.zh-CN.json`, and `meta.zh-TW.json`. Keep headings linked across locales with the
English anchor. `scripts/sync-readme.mjs` copies all three repository READMEs into `.generated/`
before development and production builds.

## Commands

```bash
npm ci               # install
npm run dev          # preview at http://localhost:3000/skit/en/
npm run build        # static production build into out/
npm run types:check  # typecheck
```
