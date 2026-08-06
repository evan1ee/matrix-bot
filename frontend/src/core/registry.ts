import { FrontendModule } from './types';

class ModuleRegistry {
  private modules: Map<string, FrontendModule> = new Map();

  register(module: FrontendModule) {
    if (this.modules.has(module.id)) {
      console.warn(`[ModuleRegistry] Module ${module.id} is already registered. Overwriting.`);
    }
    this.modules.set(module.id, module);
  }

  get(id: string): FrontendModule | undefined {
    return this.modules.get(id);
  }

  getAll(): FrontendModule[] {
    return Array.from(this.modules.values());
  }
}

export const registry = new ModuleRegistry();
