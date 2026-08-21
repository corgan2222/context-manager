import { loader } from 'fumadocs-core/source';
import { docsContentRoute, docsImageRoute, docsRoute } from './shared';
import { defineDocs } from 'fumadocs-mdx/macro';
import { metaSchema, pageSchema } from 'fumadocs-core/source/schema';
import { createElement } from 'react';
import {
  Archive,
  Bot,
  Cloud,
  GitCompare,
  Hammer,
  LayoutGrid,
  MousePointerClick,
  PencilLine,
  Plug,
  RefreshCw,
  Rocket,
  Scale,
  Sparkles,
  Star,
  Table2,
  Terminal,
} from 'lucide-react';

const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    schema: pageSchema,
    // Read from git, per file, at build time. The workflow has to check out
    // the whole history for it — with the default shallow clone every page
    // would carry the date of the deploy.
    lastModified: true,
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
  meta: {
    schema: metaSchema,
  },
});

// The icons the sidebar draws in front of each page, named in that page's
// frontmatter. Listed one by one rather than reached through the whole of
// lucide-react: a namespace import pulls every icon into the bundle, and a
// named handful is what the site actually draws. Exported because the cards on
// the documentation index draw from the same list, and two lists would drift.
export const pageIcons = {
  Archive,
  Bot,
  Cloud,
  GitCompare,
  Hammer,
  LayoutGrid,
  MousePointerClick,
  PencilLine,
  Plug,
  RefreshCw,
  Rocket,
  Scale,
  Sparkles,
  Star,
  Table2,
  Terminal,
};

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  baseUrl: docsRoute,
  source: docs.toFumadocsSource(),
  icon(icon) {
    if (icon && icon in pageIcons) {
      return createElement(pageIcons[icon as keyof typeof pageIcons], {
        className: 'size-4',
      });
    }
  },
  plugins: [],
});

export function getPageImageUrl(page: (typeof source)['$inferPage']) {
  const segments = [...page.slugs, 'image.png'];

  return {
    segments,
    url: '/' + [page.locale, ...docsImageRoute.split('/'), ...segments].filter(Boolean).join('/'),
  };
}

export function getPageMarkdownUrl(page: (typeof source)['$inferPage']) {
  const segments = [...page.slugs, 'content.md'];

  return {
    segments,
    url: '/' + [page.locale, ...docsContentRoute.split('/'), ...segments].filter(Boolean).join('/'),
  };
}

export async function getLLMText(page: (typeof source)['$inferPage']) {
  const processed = await page.data.getText('processed');

  return `# ${page.data.title} (${page.url})

${processed}`;
}
