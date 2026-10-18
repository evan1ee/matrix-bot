import React, { lazy } from 'react';
import type { FrontendModule } from '../../core/types';
import { bindQueryClient, notifyConversationUpdated } from './liveTurn';

export * from './types';
export * from './aiPromptBus';

const AiAssistantView = lazy(() =>
  import('./AiAssistantView').then((m) => ({ default: m.AiAssistantView }))
);

export const aiModule: FrontendModule = {
  id: 'ai',
  name: 'AI Assistant',
  description: 'Conversational assistant with tool calling',
  iconName: 'Sparkles',
  navTab: {
    label: 'AI Assistant',
  },
  MainView: AiAssistantView,
  handleSseEvent: (event, queryClient) => {
    bindQueryClient(queryClient);
    if (event.type === 'AiConversationUpdated' || event.type === 'AiConversationDeleted') {
      queryClient.invalidateQueries({ queryKey: ['ai-conversations'] });
      queryClient.invalidateQueries({ queryKey: ['ai-messages'] });
    }
    if (event.type === 'AiConversationUpdated') {
      const id = (event.payload as { id?: string } | null)?.id;
      if (id) notifyConversationUpdated(id);
    }
    if (event.type === 'AiConfigUpdated') {
      queryClient.invalidateQueries({ queryKey: ['ai-config'] });
    }
  },
};
