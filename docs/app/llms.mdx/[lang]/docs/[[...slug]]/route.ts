import { getLLMText, getPageMarkdownUrl, source } from '@/lib/source';
import { notFound } from 'next/navigation';

export const revalidate = false;

export async function GET(
  _req: Request,
  { params }: RouteContext<'/llms.mdx/[lang]/docs/[[...slug]]'>,
) {
  const { lang, slug } = await params;
  // Remove the appended content.md segment.
  const page = source.getPage(slug?.slice(0, -1), lang);
  if (!page) notFound();

  return new Response(await getLLMText(page), {
    headers: {
      'Content-Type': 'text/markdown',
    },
  });
}

export function generateStaticParams() {
  return source.getPages().map((page) => {
    const markdown = getPageMarkdownUrl(page);
    return { lang: markdown.lang, slug: markdown.segments };
  });
}
