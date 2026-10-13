import {
  DefaultResourceLoader, loadSkills, type ExtensionFactory, type ResourceDiagnostic,
  type SettingsManager, type Skill,
} from "@earendil-works/pi-coding-agent";
import { authorityPath } from "./files.js";

export class AuthorityResources extends DefaultResourceLoader {
  private readonly activeSkills: Skill[] = [];
  private diagnostics: ResourceDiagnostic[] = [];

  constructor(
    private readonly authorityRoot: string,
    private readonly workspace: string,
    private readonly privateAgentDir: string,
    private directories: string[],
    settings: SettingsManager,
    systemPrompt: string,
    factories: ExtensionFactory[],
  ) {
    super({
      cwd: workspace,
      agentDir: privateAgentDir,
      settingsManager: settings,
      noExtensions: true,
      noSkills: true,
      noPromptTemplates: true,
      noThemes: true,
      noContextFiles: true,
      systemPrompt,
      appendSystemPrompt: [],
      extensionFactories: factories,
    });
  }

  override getSkills(): { skills: Skill[]; diagnostics: ResourceDiagnostic[] } {
    return { skills: this.activeSkills, diagnostics: this.diagnostics };
  }

  override async reload(): Promise<void> {
    await super.reload();
    const errors = this.getExtensions().errors;
    if (errors.length > 0) throw new Error(`Worker extension load failed: ${errors.map(error => error.error).join("; ")}`);
    await this.refreshSkills();
  }

  async refreshSkills(directories = this.directories): Promise<void> {
    for (const directory of directories) await authorityPath(this.authorityRoot, directory);
    const loaded = loadSkills({
      cwd: this.workspace, agentDir: this.privateAgentDir,
      skillPaths: directories, includeDefaults: false,
    });
    if (loaded.diagnostics.length > 0) {
      throw new Error(`Skill activation failed: ${loaded.diagnostics.map(diagnostic => diagnostic.message).join("; ")}`);
    }
    for (const skill of loaded.skills) await authorityPath(this.authorityRoot, skill.filePath);
    this.directories = [...directories];
    this.diagnostics = loaded.diagnostics;
    this.activeSkills.splice(0, this.activeSkills.length, ...loaded.skills);
  }
}
