import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  // A static export has no server, so there is nothing to optimise images on
  // the way out. Without this, every page that embeds a screenshot answers
  // 500 in development and the build silently relies on the same loader.
  output: 'export',
  images: { unoptimized: true },
  reactStrictMode: true,
};

export default withMDX(config);
