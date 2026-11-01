import type { LucideIcon } from 'lucide-react';
import { Moon, Shield, Sparkles, Plus, Search } from 'lucide-react';
import { NAV_GROUPS } from '../Sidebar';
import { registry } from '../../core/registry';
import { triggerAiPrompt } from '../../modules/ai/aiPromptBus';
import type { SearchResultItem } from '../../modules/knowledge/types';

export type PaletteCategory = 'Navigation' | 'Quick actions' | 'Search';

export interface PaletteAction {
  id: string;
  title: string;
  subtitle?: string;
  category: PaletteCategory;
  icon: LucideIcon;
  /** Extra words the matcher may hit, e.g. "add" for "Create task". */
  keywords?: string[];
  run: () => void;
}

export interface PaletteContext {
  isAdmin: boolean;
  isDark: boolean;
  /** Plain navigation, as a sidebar click. */
  onSelectModule: (moduleId: string) => void;
  /** Navigation that autofocuses the target's primary input. */
  onQuickNavigate: (moduleId: string) => void;
  onToggleTheme: () => void;
}

const QUICK_JUMPS: ReadonlyArray<{ id: string; title: string; moduleId: string; keywords: string[] }> = [
  { id: 'create-todo', title: 'Create task', moduleId: 'todo', keywords: ['todo', 'add', 'new'] },
  { id: 'record-expense', title: 'Record expense', moduleId: 'finance', keywords: ['spend', 'add', 'transaction'] },
  { id: 'capture-note', title: 'Capture note', moduleId: 'knowledge', keywords: ['write', 'add', 'new'] },
  { id: 'save-bookmark', title: 'Save bookmark', moduleId: 'bookmarks', keywords: ['link', 'add', 'url'] },
  { id: 'agent-task', title: 'Trigger agent task', moduleId: 'agent', keywords: ['autonomous', 'run'] },
];

/** The types `POST /knowledge/search` can return, mapped to the module that shows them. */
const RESULT_MODULE: Record<string, string> = {
  note: 'knowledge',
  bookmark: 'bookmarks',
  journal: 'life',
  footprint: 'footprints',
};

export function buildStaticActions(ctx: PaletteContext): PaletteAction[] {
  const navigation = NAV_GROUPS.flatMap((group) =>
    group.items
      .filter((item) => registry.get(item.id))
      .map<PaletteAction>((item) => ({
        id: `nav:${item.id}`,
        title: item.label,
        subtitle: group.title,
        category: 'Navigation',
        icon: item.icon,
        run: () => ctx.onSelectModule(item.id),
      }))
  );

  if (ctx.isAdmin && registry.get('admin')) {
    navigation.push({
      id: 'nav:admin',
      title: 'Admin',
      subtitle: 'System',
      category: 'Navigation',
      icon: Shield,
      run: () => ctx.onSelectModule('admin'),
    });
  }

  const quick = QUICK_JUMPS.filter((jump) => registry.get(jump.moduleId)).map<PaletteAction>((jump) => ({
    id: `quick:${jump.id}`,
    title: jump.title,
    category: 'Quick actions',
    icon: Plus,
    keywords: jump.keywords,
    run: () => ctx.onQuickNavigate(jump.moduleId),
  }));

  quick.push({
    id: 'quick:toggle-theme',
    title: ctx.isDark ? 'Switch to light mode' : 'Switch to dark mode',
    category: 'Quick actions',
    icon: Moon,
    keywords: ['theme', 'dark', 'light', 'toggle'],
    run: ctx.onToggleTheme,
  });

  return [...navigation, ...quick];
}

export function buildSearchActions(items: readonly SearchResultItem[], ctx: PaletteContext): PaletteAction[] {
  return items.flatMap<PaletteAction>((item) => {
    const moduleId = RESULT_MODULE[item.item_type];
    if (!moduleId) return [];
    return [
      {
        id: `result:${item.item_type}:${item.item_id}`,
        title: item.title,
        subtitle: item.content_snippet,
        category: 'Search',
        icon: Search,
        run: () => ctx.onSelectModule(moduleId),
      },
    ];
  });
}

export function buildAskAiAction(query: string): PaletteAction {
  return {
    id: 'ask-ai',
    title: `Ask AI: ${query}`,
    category: 'Search',
    icon: Sparkles,
    run: () => triggerAiPrompt(query, false),
  };
}
