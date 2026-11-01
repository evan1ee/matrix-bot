import React, { useMemo } from 'react';
import { CommandPaletteModal } from './CommandPaletteModal';
import { buildStaticActions, type PaletteContext } from './paletteRegistry';

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
  context: PaletteContext;
}

export const CommandPalette: React.FC<CommandPaletteProps> = ({ isOpen, onClose, context }) => {
  const staticActions = useMemo(() => buildStaticActions(context), [context]);
  if (!isOpen) return null;
  return <CommandPaletteModal onClose={onClose} staticActions={staticActions} context={context} />;
};
