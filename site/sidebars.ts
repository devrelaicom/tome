import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docs: [
    {type: 'category', label: 'Ch. I — Getting started', collapsed: false, items: [
      'getting-started/install', 'getting-started/quickstart', 'getting-started/concepts']},
    {type: 'category', label: 'Ch. II — Using Tome', collapsed: false, items: [
      'using-tome/plugins-and-catalogs', 'using-tome/search', 'using-tome/harnesses',
      'using-tome/workspaces', 'using-tome/meta-skills', 'using-tome/mcp-server',
      'using-tome/troubleshooting']},
    {type: 'category', label: 'Ch. III — Authoring', collapsed: false, items: [
      'authoring/overview', 'authoring/create', 'authoring/convert', 'authoring/lint',
      'authoring/distributing']},
    {type: 'category', label: 'Ch. IV — Reference', items: [
      'reference/commands', 'reference/exit-codes', 'reference/config',
      'reference/security-model']},
  ],
};

export default sidebars;
