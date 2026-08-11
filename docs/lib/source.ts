import { docs } from 'collections/server';
import { loader } from 'fumadocs-core/source';
import { i18n } from './i18n';
import { docsContentRoute, docsImageRoute, docsRoute } from './shared';

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  i18n,
  baseUrl: docsRoute,
  source: docs.toFumadocsSource(),
  plugins: [],
});

export function getPageImage(page: (typeof source)['$inferPage']) {
  const locale = page.locale ?? i18n.defaultLanguage;
  const segments = [...page.slugs, 'image.png'];

  return {
    lang: locale,
    segments,
    url: `${docsImageRoute(locale)}/${segments.join('/')}`,
  };
}

export function getPageMarkdownUrl(page: (typeof source)['$inferPage']) {
  const locale = page.locale ?? i18n.defaultLanguage;
  const segments = [...page.slugs, 'content.md'];

  return {
    lang: locale,
    segments,
    url: `${docsContentRoute(locale)}/${segments.join('/')}`,
  };
}

export async function getLLMText(page: (typeof source)['$inferPage']) {
  const processed = await page.data.getText('processed');

  return `# ${page.data.title} (${page.url})

${processed}`;
}
