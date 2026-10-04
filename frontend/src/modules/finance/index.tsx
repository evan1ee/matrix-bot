import React, { lazy } from 'react';
import type { FrontendModule } from '../../core/types';

export * from './types';
export * from './utils';

const FinanceMainView = lazy(() =>
  import('./FinanceMainView').then((m) => ({ default: m.FinanceMainView }))
);

export { FinanceMainView };

export const financeModule: FrontendModule = {
  id: 'finance',
  name: 'Expenses & Income',
  description: 'Track spending and income per currency, with your own categories',
  navTab: { label: 'Finance' },
  MainView: FinanceMainView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'TransactionCreated' ||
      event.type === 'TransactionUpdated' ||
      event.type === 'TransactionDeleted'
    ) {
      queryClient.invalidateQueries({ queryKey: ['transactions'] });
      queryClient.invalidateQueries({ queryKey: ['finance-summary'] });
      queryClient.invalidateQueries({ queryKey: ['finance-analytics'] });
      queryClient.invalidateQueries({ queryKey: ['net-worth'] });
    }
    if (event.type === 'CategoryCreated' || event.type === 'CategoryDeleted') {
      queryClient.invalidateQueries({ queryKey: ['finance-categories'] });
      queryClient.invalidateQueries({ queryKey: ['finance-analytics'] });
    }
  },
};
