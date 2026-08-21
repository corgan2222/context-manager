import type { Viewport } from 'next';
import { Inter } from 'next/font/google';
import { Provider } from '@/components/provider';
import './global.css';

const inter = Inter({
  subsets: ['latin'],
});

// `colorScheme` renders as <meta name="color-scheme" content="dark">. It is
// what tells a browser to draw form controls and scrollbars dark, and what a
// page-darkening extension reads before deciding the page needs its help.
export const viewport: Viewport = {
  colorScheme: 'dark',
};

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    // `dark` written into the markup rather than left to the provider: the
    // site is a static export, so this file is the first frame a reader gets,
    // and a page that starts light and turns dark is a page that flashes.
    <html
      lang="en"
      className={`dark ${inter.className}`}
      style={{ colorScheme: 'dark' }}
      suppressHydrationWarning
    >
      <body className="flex flex-col min-h-screen">
        <Provider>{children}</Provider>
      </body>
    </html>
  );
}
