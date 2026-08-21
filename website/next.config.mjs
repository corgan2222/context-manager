import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  // A static export has no server, so there is nothing to optimise images on
  // the way out. Without this, every page that embeds a screenshot answers
  // 500 in development and the build silently relies on the same loader.
  output: 'export',
  images: { unoptimized: true },
  // GitHub Pages serves a project site under /<repo>, not at the root, so
  // every link and asset needs the prefix. Set here rather than only in the
  // workflow: a path that is right in CI and wrong locally is a path nobody
  // tests. `npm run dev` therefore also runs under /context-manager.
  basePath: '/context-manager',
  reactStrictMode: true,
};

export default withMDX(config);
