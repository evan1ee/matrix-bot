import { useEffect, useSyncExternalStore } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { bindQueryClient, clearTurn, getSnapshot, startTurn, stopTurn, subscribe } from './liveTurn';

/** The live AI turn and the actions on it. The turn outlives this hook's component. */
export function useAiTurn() {
  const queryClient = useQueryClient();
  useEffect(() => {
    bindQueryClient(queryClient);
  }, [queryClient]);
  const turn = useSyncExternalStore(subscribe, getSnapshot);
  return { turn, startTurn, stopTurn, clearTurn };
}
