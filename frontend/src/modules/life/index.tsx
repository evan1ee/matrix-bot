import React, { lazy } from 'react';
import type { FrontendModule, SystemEvent } from '../../core/types';

export * from './types';

const LifeView = lazy(() =>
  import('./LifeView').then((m) => ({ default: m.LifeView }))
);

export { LifeView };

export const lifeModule: FrontendModule = {
  id: 'life',
  name: 'Life Grid',
  description: 'Lifespan Day-by-Day Visualizer and Daily Reflection Grid',
  iconName: 'Hourglass',
  navTab: {
    label: 'Life Grid',
  },
  MainView: LifeView,
  handleSseEvent: (event: SystemEvent, queryClient) => {
    if (
      event.type === 'LifeProfileUpdated' ||
      event.type === 'LifeDayLogged' ||
      event.type === 'LifeDayCleared' ||
      event.type.startsWith('Todo') ||
      event.type.startsWith('Transaction') ||
      event.type.startsWith('Habit') ||
      event.type.startsWith('Bookmark') ||
      event.type.startsWith('Footprint') ||
      event.type.startsWith('Reminder')
    ) {
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    }
  },
};
