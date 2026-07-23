import { i18n } from '@/lib/i18n';
import { RedirectToDocs } from './redirect-to-docs';

export function generateStaticParams() {
  return i18n.languages.map((lang) => ({ lang }));
}

export default async function HomePage({
  params,
}: {
  params: Promise<{ lang: string }>;
}) {
  const { lang } = await params;
  return <RedirectToDocs lang={lang} />;
}
