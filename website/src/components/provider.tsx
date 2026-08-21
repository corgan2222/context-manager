'use client';
import SearchDialog from '@/components/search';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ReactNode } from 'react';

export function Provider({ children }: { children: ReactNode }) {
  return (
    // One theme, dark, no switch and no reading of the system setting. The
    // screenshots the site is built around are dark, and a light page around
    // them reads as a mistake. Extensions that darken pages themselves — Dark
    // Reader and its kind — decide from `color-scheme`, which `<html>` and
    // the metadata declare; without that they keep switching themselves on
    // over a page that was already dark.
    <RootProvider
      search={{ SearchDialog }}
      theme={{ forcedTheme: 'dark', defaultTheme: 'dark', enableSystem: false }}
    >
      {children}
    </RootProvider>
  );
}
