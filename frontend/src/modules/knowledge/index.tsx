import React from 'react';
import type { FrontendModule, SystemEvent } from '../../core/types';

const LazyKnowledgeMainView = React.lazy(() =>
  import('./KnowledgeMainView').then((m) => ({ default: m.KnowledgeMainView }))
);

export const KnowledgeView: React.FC = () => (
  <React.Suspense
    fallback={
      <div className="py-16 text-center text-xs text-neutral-400 font-mono">
        Loading notes...
      </div>
    }
  >
    <LazyKnowledgeMainView />
  </React.Suspense>
);

export const knowledgeModule: FrontendModule = {
  id: 'knowledge',
  name: 'Notes',
  description: 'Personal knowledge base, notes, and semantic search',
  iconName: 'Brain',
  navTab: {
    label: 'Notes',
  },
  MainView: KnowledgeView,
  handleSseEvent: (event: SystemEvent, queryClient) => {
    if (
      event.type === 'NoteCreated' ||
      event.type === 'NoteUpdated' ||
      event.type === 'NoteDeleted' ||
      event.type === 'KnowledgeEmbeddingIndexed'
    ) {
      queryClient.invalidateQueries({ queryKey: ['notes'] });
      queryClient.invalidateQueries({ queryKey: ['notes-tags'] });
      queryClient.invalidateQueries({ queryKey: ['knowledge-search'] });
    }
  },
};
