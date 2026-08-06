import { ReactNode } from 'react';

export interface StatItem {
  id: string;
  label: string;
  value: string | number;
  detail?: string;
  status?: 'default' | 'success' | 'warning' | 'error';
}

export interface SystemEvent {
  type: string;
  payload: any;
  timestamp: string;
}

export interface FrontendModule {
  id: string;
  name: string;
  description?: string;
  iconName?: string;
  
  // Dynamic slot components
  navTab?: {
    label: string;
    badgeCount?: number;
  };
  
  getStatItems?: (queryClient: any) => StatItem[];
  
  MainView: React.ComponentType<{
    activeRoomId?: string;
    onSelectRoom?: (roomId: string) => void;
    events?: SystemEvent[];
    isConnected?: boolean;
  }>;
  
  handleSseEvent?: (event: SystemEvent, queryClient: any) => void;
}
