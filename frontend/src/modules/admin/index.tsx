import { FrontendModule } from '../../core/types';
import { AdminView } from './AdminModule';

export * from './types';
export * from './AdminModule';

export const adminModule: FrontendModule = {
  id: 'admin',
  name: 'Admin Console',
  description: 'Runtime configuration and encrypted credentials vault',
  iconName: 'Shield',
  navTab: {
    label: 'Admin',
  },
  MainView: AdminView,
};
