'use client';
import Link from 'next/link';
import { useEffect } from 'react';

// There is no separate landing page — the docs Overview (the rendered README) is
// the front door. This locale-root page forwards there; the link is the no-JS
// fallback. basePath is inlined at build time so the target is absolute and
// independent of the current trailing slash.
const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '/skit';

export function RedirectToDocs({ lang }: { lang: string }) {
  useEffect(() => {
    window.location.replace(`${basePath}/${lang}/docs/`);
  }, [lang]);

  return (
    <main className="flex flex-1 flex-col items-center justify-center px-4 text-center">
      <p className="text-fd-muted-foreground">
        Redirecting to the{' '}
        <Link href={`/${lang}/docs`} className="text-fd-foreground underline">
          documentation
        </Link>
        …
      </p>
    </main>
  );
}
