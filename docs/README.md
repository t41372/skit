# docs

This directory is two things:

- **The documentation site** — a [Fumadocs](https://fumadocs.dev) app (Next.js
  static export), deployed to GitHub Pages at https://t41372.github.io/skit/ by
  `.github/workflows/docs.yml` on pushes to main.
- **Repo doc assets** — `assets/` (media the READMEs hotlink via
  raw.githubusercontent; the demo `.mp4`s are deliberately untracked) and
  `design/` (internal design notes). Neither is published to the site.

## Editing content

Pages are the MDX files in `content/docs/`; sidebar order lives in
`content/docs/meta.json`. English-only for now — the i18n structure is ready
(`lib/i18n.ts`), no translated content exists yet.

## Commands

```bash
npm ci               # install
npm run dev          # preview at http://localhost:3000/skit/en/
npm run build        # static production build into out/
npm run types:check  # typecheck
```
