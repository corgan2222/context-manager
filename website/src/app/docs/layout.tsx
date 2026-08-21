import { source } from '@/lib/source';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { baseOptions } from '@/lib/layout.shared';

export default function Layout({ children }: LayoutProps<'/docs'>) {
  const { githubUrl: _githubUrl, ...base } = baseOptions();

  return (
    // No footer under the page list. fumadocs draws one as soon as it has
    // something to put there — the theme switch, or an icon link built from
    // `githubUrl` — so both are dropped here rather than hidden with CSS. Both
    // still sit in the header of the landing page, which is where a reader
    // looks for them.
    <DocsLayout
      tree={source.getPageTree()}
      {...base}
      themeSwitch={{ enabled: false }}
    >
      {children}
    </DocsLayout>
  );
}
