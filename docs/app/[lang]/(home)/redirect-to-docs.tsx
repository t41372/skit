'use client';
import Link from 'next/link';
import { useEffect } from 'react';

// There is no separate landing page — the docs Overview (the rendered README) is
// the front door. This locale-root page forwards there; the link is the no-JS
// fallback. basePath is inlined at build time so the target is absolute and
// independent of the current trailing slash.
const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '/skit';

// One short sentence per locale, split around the link text.
const notices: Record<string, { before: string; link: string; after: string }> = {
  en: { before: 'Redirecting to the ', link: 'documentation', after: '…' },
  'zh-CN': { before: '正在跳转到', link: '文档', after: '…' },
  'zh-TW': { before: '正在跳轉到', link: '文件', after: '…' },
};

export function RedirectToDocs({ lang }: { lang: string }) {
  useEffect(() => {
    window.location.replace(`${basePath}/${lang}/docs/`);
  }, [lang]);

  const notice = notices[lang] ?? notices.en;

  return (
    <main className="flex flex-1 flex-col items-center justify-center px-4 text-center">
      <p className="text-fd-muted-foreground">
        {notice.before}
        <Link href={`/${lang}/docs`} className="text-fd-foreground underline">
          {notice.link}
        </Link>
        {notice.after}
      </p>
    </main>
  );
}
