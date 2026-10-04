import React, { lazy } from 'react';
import type { FrontendModule } from '../../core/types';

export * from './types';
export * from './utils';

const PortfolioMainView = lazy(() =>
  import('./PortfolioMainView').then((m) => ({ default: m.PortfolioMainView }))
);

export { PortfolioMainView };

export const portfolioModule: FrontendModule = {
  id: 'portfolio',
  name: 'Portfolio',
  description: 'Track investment holdings, cost basis, and unrealized P&L per currency',
  navTab: { label: 'Portfolio' },
  MainView: PortfolioMainView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'PortfolioHoldingCreated' ||
      event.type === 'PortfolioHoldingUpdated' ||
      event.type === 'PortfolioHoldingDeleted' ||
      event.type === 'PortfolioAuditLogged' ||
      event.type === 'PortfolioAlertCreated' ||
      event.type === 'PortfolioAlertTriggered' ||
      event.type === 'PortfolioAlertDeleted'
    ) {
      queryClient.invalidateQueries({ queryKey: ['portfolio-holdings'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-summary'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio', 'activity'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-alerts'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-history'] });
      queryClient.invalidateQueries({ queryKey: ['net-worth'] });
    }
    if (event.type === 'PortfolioCashUpdated') {
      queryClient.invalidateQueries({ queryKey: ['portfolio-cash'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-summary'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio', 'activity'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-history'] });
      queryClient.invalidateQueries({ queryKey: ['net-worth'] });
    }
  },
};
