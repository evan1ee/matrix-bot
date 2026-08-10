import { QueryClient } from '@tanstack/react-query';

export interface StatItem {
  id: string;
  label: string;
  value: string | number;
  detail?: string;
  status?: 'default' | 'success' | 'warning' | 'error';
}

export interface SystemEvent {
  /// The Rust `SystemEvent` variant name, e.g. `TodoCreated`.
  type: string;
  payload: unknown;
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
  
  getStatItems?: (queryClient: QueryClient) => StatItem[];
  
  MainView: React.ComponentType<{
    activeRoomId?: string;
    onSelectRoom?: (roomId: string) => void;
    events?: SystemEvent[];
    isConnected?: boolean;
    onNavigate?: (moduleId: string) => void;
    autoFocus?: boolean;
  }>;
  
  handleSseEvent?: (event: SystemEvent, queryClient: QueryClient) => void;
}
