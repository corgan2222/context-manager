import { Callout } from 'fumadocs-ui/components/callout';
import { Card, Cards } from 'fumadocs-ui/components/card';
import {
  ImageZoom,
  type ImageZoomProps,
} from 'fumadocs-ui/components/image-zoom';
import { Step, Steps } from 'fumadocs-ui/components/steps';
import { Tab, Tabs } from 'fumadocs-ui/components/tabs';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import { pageIcons } from '@/lib/source';
import type { MDXComponents } from 'mdx/types';
import type { ReactNode } from 'react';

/* The three marks the comparison table is made of. Components rather than
 * characters typed into the cells: colour is what makes a table of 112 cells
 * readable at a glance, and a green tick pasted as an emoji renders as
 * whatever the reader's system happens to have. */
function Yes() {
  return (
    <svg
      viewBox="0 0 20 20"
      className="inline-block size-4 text-emerald-500"
      fill="currentColor"
      aria-label="yes"
      role="img"
    >
      <path
        fillRule="evenodd"
        d="M16.7 5.3a1 1 0 0 1 0 1.4l-7.5 7.5a1 1 0 0 1-1.4 0L3.3 9.7a1 1 0 1 1 1.4-1.4l3.8 3.8 6.8-6.8a1 1 0 0 1 1.4 0Z"
        clipRule="evenodd"
      />
    </svg>
  );
}

function No() {
  return (
    <svg
      viewBox="0 0 20 20"
      className="inline-block size-4 text-red-500"
      fill="currentColor"
      aria-label="no"
      role="img"
    >
      <path
        fillRule="evenodd"
        d="M5.3 5.3a1 1 0 0 1 1.4 0L10 8.6l3.3-3.3a1 1 0 1 1 1.4 1.4L11.4 10l3.3 3.3a1 1 0 0 1-1.4 1.4L10 11.4l-3.3 3.3a1 1 0 0 1-1.4-1.4L8.6 10 5.3 6.7a1 1 0 0 1 0-1.4Z"
        clipRule="evenodd"
      />
    </svg>
  );
}

/** Present, but limited or only through a workaround. */
function Part({ children }: { children?: ReactNode }) {
  return (
    <span className="text-amber-500" title="limited, or needs a workaround">
      {children ?? 'partly'}
    </span>
  );
}

// Registered globally rather than imported per page: a page that has to
// remember an import is a page somebody will write without one, and the build
// only says so at export time.
export function getMDXComponents(components?: MDXComponents) {
  return {
    ...defaultMdxComponents,
    // Every screenshot is 2400x1500 and the column shows it at about a third
    // of that. Mapping `img` rather than asking each page to use <ImageZoom>
    // means a picture written as ordinary Markdown is zoomable too. The cast
    // is needed because MDX types `img` as a plain <img>, while ImageZoom
    // takes the wider set of props next/image accepts.
    img: (props) => <ImageZoom {...(props as ImageZoomProps)} />,
    Callout,
    Card,
    Cards,
    Yes,
    No,
    Part,
    // So a card on the index can carry the icon its page carries in the
    // sidebar, written as <LayoutGrid /> without an import per page.
    ...pageIcons,
    Step,
    Steps,
    Tab,
    Tabs,
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
