# Command palette (Blueprint 9)

Spec: `specs/2026-10-31-feature-blueprints.md` § Blueprint 9. Frontend only; no backend, schema or OpenAPI change.

## Rulings

- *Ruling: instant search calls the existing `POST /knowledge/search` (via `useKnowledgeSearch`, 200 ms debounce), not the blueprint's `GET /api/v1/search/hybrid`, which does not exist. Since Blueprint 8 that endpoint already ranks notes, bookmarks, journal and footprints together — cost if wrong: none.*
- *Ruling: navigation entries come from `Sidebar`'s exported `NAV_GROUPS` (filtered to registered modules, `admin` only for admins), not a new list — one source of truth for labels and icons.*
- *Ruling: quick actions reuse the existing Quick Action jump (`onQuickNavigate`, which autofocuses the target's primary input) instead of opening Add* modals from outside their module. Ask AI uses `triggerAiPrompt`; theme toggle uses the existing theme handler.*
- *Ruling: no unit-test runner is added (AGENTS.md rule 4); the pure matcher is tested by a DOM-free Playwright spec.*

## Steps

1. `paletteMatch.ts` — pure scoring (exact > prefix > word-prefix > substring > subsequence), stable order. Test first.
2. `paletteRegistry.ts` — builds navigation + quick actions + search-result actions.
3. `useCommandPalette.ts` — Ctrl/Cmd+K toggle, open state.
4. `CommandPaletteModal.tsx` — ARIA combobox dialog, focus trap, Escape restores focus, arrows/Enter.
5. Mount in `App.tsx`; add a sidebar hint button.
6. Playwright `e2e/command-palette.spec.ts`; gates: `npm run build`, `npm run test:e2e`, `typecheck:e2e`.
