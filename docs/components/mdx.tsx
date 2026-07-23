import defaultMdxComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';

export function getMDXComponents(components?: MDXComponents) {
  return {
    ...defaultMdxComponents,
    // Render a plain <img>, not next/image. The README (rendered on the landing
    // page) uses remote GitHub-hosted images and a `<img width=480>` GIF with no
    // height, which next/image rejects — and with `images.unoptimized` a static
    // export gains nothing from next/image anyway.
    img: (props: React.ImgHTMLAttributes<HTMLImageElement>) => (
      // eslint-disable-next-line @next/next/no-img-element
      <img loading="lazy" {...props} alt={props.alt ?? ''} />
    ),
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
