import { defineConfig, defineDocs } from 'fumadocs-mdx/config';
import { metaSchema, pageSchema } from 'fumadocs-core/source/schema';
import rehypeRaw from 'rehype-raw';

const GITHUB_BLOB = 'https://github.com/t41372/skit/blob/main/';

// Each locale's landing page <include>s the matching repo README, whose relative
// links (./README.zh-TW.md, LICENSE, ./CONTRIBUTING.md) point at repo files that
// do not exist on the site. Rewrite them to GitHub. This rule applies to the landing pages
// so route-style links elsewhere stay untouched.
const LANDING_PAGE = /content\/docs\/index(\.[\w-]+)?\.mdx$/;

function remarkReadmeRelativeLinks() {
  return (tree: unknown, file: { path?: string }) => {
    const path = String(file.path ?? '').replaceAll('\\', '/');
    if (!LANDING_PAGE.test(path)) return;
    const walk = (node: {
      type?: string;
      url?: string;
      children?: unknown[];
    }) => {
      if (
        node.type === 'link' &&
        typeof node.url === 'string' &&
        !/^(https?:|mailto:|\/|#)/.test(node.url)
      ) {
        node.url = GITHUB_BLOB + node.url.replace(/^\.\//, '');
      }
      for (const child of node.children ?? []) walk(child as never);
    };
    walk(tree as never);
  };
}

// You can customize Zod schemas for frontmatter and `meta.json` here
// see https://fumadocs.dev/docs/mdx/collections
export const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    schema: pageSchema,
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
  meta: {
    schema: metaSchema,
  },
});

export default defineConfig({
  mdxOptions: {
    remarkPlugins: (defaults) => [...defaults, remarkReadmeRelativeLinks],
    // The landing page <include>s the repo README, which carries raw HTML (the
    // hero <video>, a centered <img> block). That HTML arrives as `raw` nodes
    // the MDX compiler cannot serialize. rehype-raw parses them into real
    // elements. This is the MDX-documented recipe: the passThrough list keeps
    // MDX's own node types intact, and running it before the default rehype
    // plugins keeps Shiki's output untouched.
    rehypePlugins: (defaults) => [
      [
        rehypeRaw,
        {
          passThrough: [
            'mdxjsEsm',
            'mdxFlowExpression',
            'mdxJsxFlowElement',
            'mdxJsxTextElement',
            'mdxTextExpression',
          ],
        },
      ],
      ...defaults,
    ],
  },
});
