import { source } from '@/lib/source';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { baseOptions } from '@/lib/layout.shared';

export default function Layout({ children }: LayoutProps<'/docs'>) {
  const { githubUrl: _githubUrl, ...base } = baseOptions();

  return (
    // No footer under the page list. fumadocs draws one as soon as it has
    // something to put there, and the icon link built from `githubUrl` is the
    // last thing left that would fill it. The link still sits in the header of
    // the landing page.
    <DocsLayout tree={source.getPageTree()} {...base}>
      {children}
    </DocsLayout>
  );
}
