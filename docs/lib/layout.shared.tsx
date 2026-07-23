import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { uiTranslations } from 'fumadocs-ui/i18n';
import { i18n } from './i18n';
import { appName, gitConfig } from './shared';

// Base path under which the static site is served (see next.config.mjs). Needed
// to reference the site icon from raw markup, which doesn't get the automatic
// basePath prefix that next/link / metadata icons do.
const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '/skit';

// Translations for Fumadocs' own layout chrome. English ships via the official
// `uiTranslations()` pack; register per-locale overrides here as locales grow.
export const translations = i18n.translations().extend(uiTranslations());

export function baseOptions(locale: string): BaseLayoutProps {
  return {
    nav: {
      // The docs Overview is the front door — there's no separate landing page,
      // so the logo links straight there.
      url: `/${locale}/docs`,
      title: (
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: '0.4rem' }}>
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={`${basePath}/icon.png`} alt="" width={22} height={22} />
          {appName}
        </span>
      ),
    },
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
  };
}
