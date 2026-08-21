import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import Image from 'next/image';
import icon from '@/assets/icon.png';
import { appName, gitConfig } from './shared';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      // The icon the executable carries, so the header shows the same thing
      // the taskbar does. Imported rather than referenced from public/: with
      // `images.unoptimized` a plain src keeps the path it was written with,
      // which drops the /context-manager prefix and answers 404. A static
      // import goes through the bundler, which knows the prefix.
      title: (
        <>
          <Image src={icon} alt="" width={22} height={22} className="rounded-[5px]" />
          {appName}
        </>
      ),
    },
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
  };
}
