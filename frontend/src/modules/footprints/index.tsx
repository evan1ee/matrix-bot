import React, { lazy } from 'react';
import type { FrontendModule } from '../../core/types';

export * from './types';

const FootprintsView = lazy(() =>
  import('./FootprintsView').then((m) => ({ default: m.FootprintsView }))
);

export const footprintsModule: FrontendModule = {
  id: 'footprints',
  name: 'Footprints',
  description: 'Track visited places on an interactive star map with memories and spending',
  iconName: 'Compass',
  navTab: {
    label: 'Footprints',
  },
  MainView: FootprintsView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'FootprintCreated' ||
      event.type === 'FootprintUpdated' ||
      event.type === 'FootprintDeleted'
    ) {
      queryClient.invalidateQueries({ queryKey: ['footprints'] });
      queryClient.invalidateQueries({ queryKey: ['footprints-stats'] });
    }
  },
};
