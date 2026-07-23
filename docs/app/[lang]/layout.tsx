import { Inter } from 'next/font/google';
import { i18nProvider } from 'fumadocs-ui/i18n';
import type { ReactNode } from 'react';
import { Provider } from '@/components/provider';
import { i18n } from '@/lib/i18n';
import { translations } from '@/lib/layout.shared';
import '../global.css';

const inter = Inter({
  subsets: ['latin'],
});

export function generateStaticParams() {
  return i18n.languages.map((lang) => ({ lang }));
}

export default async function Layout({
  params,
  children,
}: {
  params: Promise<{ lang: string }>;
  children: ReactNode;
}) {
  const { lang } = await params;

  return (
    <html lang={lang} className={inter.className} suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <Provider i18n={i18nProvider(translations, lang)}>{children}</Provider>
      </body>
    </html>
  );
}
