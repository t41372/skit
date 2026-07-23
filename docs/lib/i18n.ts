import { defineI18n } from 'fumadocs-core/i18n';

// The i18n foundation. English is the default and only shipped locale today.
//
// To add a locale later (e.g. zh-TW, zh-CN) you only:
//   1. add its code to `languages` below, and
//   2. drop translated content files next to the English ones
//      (`content/docs/index.zh-TW.mdx`, `meta.zh-TW.json`, ...).
// No route or folder restructuring is needed — the `app/[lang]` tree already
// renders every locale.
//
// Every locale keeps its URL prefix (`/en/docs`, `/zh-TW/docs`, ...). Hiding the
// default locale's prefix needs a runtime proxy/middleware that a static export
// cannot run, so we don't pretend otherwise: `public/index.html` redirects the
// site root to `/en/`.
export const i18n = defineI18n({
  defaultLanguage: 'en',
  languages: ['en'],
  hideLocale: 'never',
});
