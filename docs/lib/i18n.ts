import { defineI18n } from 'fumadocs-core/i18n';

// English is the source locale. Simplified and Traditional Chinese are complete.
//
// Use these steps to add another locale:
//   1. add its code to `languages` below,
//   2. add translated content files next to the English files
//      (`content/docs/index.<code>.mdx`, `meta.<code>.json`, ...), and
//   3. register its Fumadocs chrome strings in `lib/layout.shared.tsx`.
// The `app/[lang]` tree already renders each locale. Do not change the routes or folders.
//
// Each locale keeps its URL prefix, such as `/en/docs` or `/zh-TW/docs`. A static export cannot
// hide the default prefix. The `public/index.html` file redirects the site root to `/en/`.
export const i18n = defineI18n({
  defaultLanguage: 'en',
  languages: ['en', 'zh-CN', 'zh-TW'],
  hideLocale: 'never',
});
