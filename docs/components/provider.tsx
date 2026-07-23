'use client';
import SearchDialog from '@/components/search';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ComponentProps, ReactNode } from 'react';

export function Provider({
  children,
  i18n,
}: {
  children: ReactNode;
  i18n?: ComponentProps<typeof RootProvider>['i18n'];
}) {
  return (
    <RootProvider search={{ SearchDialog }} i18n={i18n}>
      {children}
    </RootProvider>
  );
}
