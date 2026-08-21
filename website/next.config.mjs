import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  // A static export has no server, so there is nothing to optimise images on
  // the way out. Without this, every page that embeds a screenshot answers
  // 500 in development and the build silently relies on the same loader.
  output: 'export',
  images: { unoptimized: true },
  // `next dev` and `next build` both write here, and a build run while the
  // dev server is up leaves it reading half a cache: the symptom is
  // `Cannot find module 'nanoid/non-secure'` from postcss and a 500 on
  // global.css. Set NEXT_DIST_DIR for the build to give it its own directory,
  // and the dev server keeps running: NEXT_DIST_DIR=.next-build npm run build
  //
  // The export follows distDir, so with the variable set the finished site
  // lands in .next-build/ rather than in out/. Whoever checks the export by
  // hand looks there; CI sets nothing and keeps writing to out/.
  distDir: process.env.NEXT_DIST_DIR ?? '.next',
  // GitHub Pages serves a project site under /<repo>, not at the root, so
  // every link and asset needs the prefix. Set here rather than only in the
  // workflow: a path that is right in CI and wrong locally is a path nobody
  // tests. `npm run dev` therefore also runs under /context-manager.
  basePath: '/context-manager',
  // Without this the export writes docs.html and no docs/index.html, so a
  // typed or shared /docs/ answers 404 while /docs answers 200. Static hosts
  // resolve a directory to its index, and GitHub Pages redirects the form
  // without the slash to the one with it, so writing index files serves both.
  trailingSlash: true,
  reactStrictMode: true,
};

export default withMDX(config);
