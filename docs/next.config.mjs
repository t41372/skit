import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

// The site is served from https://t41372.github.io/skit/, so every route and
// asset is prefixed with `/skit`. Override with NEXT_PUBLIC_BASE_PATH='' to build
// for a root-served host or to preview locally at `/`.
const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '/skit';

/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  reactStrictMode: true,
  // basePath handles both routing and asset prefixing under `/skit`.
  basePath: basePath || undefined,
  // Directory-style URLs (`/docs/`) serve cleanly from a static file host.
  trailingSlash: true,
  // No Image Optimization server exists in a static export.
  images: { unoptimized: true },
  // In production the site root redirects to /en/ via public/index.html, but
  // `next dev` doesn't serve that file, leaving the root a 404. Mirror the
  // redirect here — dev only, because `output: 'export'` builds reject
  // `redirects()`.
  ...(process.env.NODE_ENV === 'development'
    ? {
        redirects: async () => [
          { source: '/', destination: '/en/docs/', permanent: false },
        ],
      }
    : {}),
};

export default withMDX(config);
