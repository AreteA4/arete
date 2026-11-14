import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightLlmsTxt from "starlight-llms-txt";
import { ecVersionPlugin } from "./src/plugins/ec-version-plugin.mjs";
import { remarkVersion } from "./src/plugins/remark-version.mjs";

export default defineConfig({
  markdown: {
    remarkPlugins: [remarkVersion],
  },
  site: "https://docs.arete.run",
  integrations: [
    starlight({
      expressiveCode: {
        plugins: [ecVersionPlugin()],
      },
      plugins: [
        starlightLlmsTxt({
          projectName: "Arete",
          description:
            "Arete gives AI agents sandboxes on Solana: a funded wallet the agent can use but never holds the key to, inside rules its owner signs. The rules decide which programs the agent may act on and read; program reads and live views are the sandbox's read side. Protocols, automations, cloud dev environments and SDKs are in private beta.",
          promote: [
            "sandboxes/overview",
            "sandboxes/quickstart",
            "sandboxes/how-it-works",
            "sandboxes/rules",
            "sandboxes/actions",
            "sandboxes/accounts",
            "sandboxes/agents",
            "sandboxes/security",
            "sandboxes/read-scope",
            "sandboxes/connect-agent",
            "sandboxes/skill",
            "sandboxes/errors",
            "using-programs/program-reads",
            "using-stacks/connect",
          ],
        }),
      ],
      title: "Arete",
      favicon: "/favicon.ico",
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/AreteA4/arete",
        },
      ],
      customCss: ["./src/styles/custom.css"],
      // Component overrides for custom design and analytics
      components: {
        Sidebar: "./src/components/overrides/Sidebar.astro",
        SiteTitle: "./src/components/overrides/SiteTitle.astro",
        ThemeProvider: "./src/components/overrides/ThemeProvider.astro",
        ThemeSelect: "./src/components/overrides/ThemeSelect.astro",
        EditLink: "./src/components/overrides/EditLink.astro",
        Head: "./src/components/overrides/Head.astro",
        Footer: "./src/components/overrides/Footer.astro",
        MarkdownContent: "./src/components/overrides/MarkdownContent.astro",
        Search: "./src/components/overrides/Search.astro",
        PageTitle: "./src/components/overrides/PageTitle.astro",
      },
      // Keep existing slugs stable. Sandboxes lead; their read side (program
      // reads, live views) sits beside them; everything else is private beta.
      sidebar: [
        {
          label: "Start Here",
          items: [
            { label: "Overview", link: "/" },
            { slug: "sandboxes/overview" },
            { slug: "sandboxes/quickstart" },
            { slug: "sandboxes/how-it-works" },
            { slug: "sandboxes/use-cases" },
          ],
        },
        {
          label: "Sandboxes",
          items: [
            { slug: "sandboxes/funding" },
            { slug: "sandboxes/agents" },
            { slug: "sandboxes/rules" },
            { slug: "sandboxes/actions" },
            { slug: "sandboxes/accounts" },
            { slug: "sandboxes/history" },
            { slug: "sandboxes/security" },
          ],
        },
        {
          label: "Reading in a Sandbox",
          items: [
            { slug: "sandboxes/read-scope" },
            {
              label: "Program Reads",
              items: [
                { slug: "using-programs/overview" },
                { slug: "using-programs/program-reads" },
                { slug: "using-programs/chain-reads" },
              ],
            },
            {
              label: "Live Views",
              items: [
                { slug: "using-stacks/how-it-works" },
                { slug: "using-stacks/connect" },
                { slug: "using-stacks/filtering-feeds" },
              ],
            },
            {
              label: "Discovery",
              items: [
                { slug: "concepts/programs-views-stacks" },
                { slug: "agent-skills/explore" },
                { slug: "agent-skills/explore-on-chain" },
              ],
            },
          ],
        },
        {
          label: "Connect Your Agent",
          items: [
            { slug: "sandboxes/connect-agent" },
            { slug: "sandboxes/skill" },
          ],
        },
        {
          label: "Reference",
          items: [
            { slug: "sandboxes/errors" },
            { slug: "agent-skills/mcp" },
            { slug: "cli/commands" },
            { slug: "reference/api-problems" },
            { slug: "getting-started/what-is-arete" },
          ],
        },
        {
          label: "Private Beta",
          collapsed: true,
          items: [
            { slug: "private-beta" },
            {
              label: "Protocols",
              collapsed: true,
              items: [
                { slug: "concepts/protocols" },
                { slug: "using-stacks/quickstart" },
                { slug: "getting-started/from-question-to-app" },
                { slug: "building-protocols/definition" },
                { slug: "building-protocols/lifecycle" },
                { slug: "building-protocols/handlers" },
                { slug: "building-protocols/automation" },
                { slug: "building-protocols/patterns" },
                { slug: "building-protocols/tutorial-lifecycle" },
                { slug: "building-protocols/squads" },
                { slug: "reference/protocols" },
                { slug: "reference/protocol-expressions" },
              ],
            },
            {
              label: "Automations",
              collapsed: true,
              items: [
                { slug: "automations/overview" },
                { slug: "automations/quickstart" },
                { slug: "automations/triggers" },
                { slug: "automations/consent" },
                { slug: "automations/signing" },
                { slug: "automations/production" },
              ],
            },
            {
              label: "Develop and Test",
              collapsed: true,
              items: [
                { slug: "cli/local-development" },
                { slug: "development/application" },
                { slug: "development/testing" },
              ],
            },
            {
              label: "Integrate Protocols",
              collapsed: true,
              items: [
                { slug: "integrating-protocols/overview" },
                { slug: "integrating-protocols/execution" },
                { slug: "integrating-protocols/react" },
                { slug: "integrating-protocols/events" },
                { slug: "agent-skills/protocols" },
              ],
            },
            {
              label: "Deploy and Publish",
              collapsed: true,
              items: [
                { slug: "deploying-protocols/overview" },
                { slug: "deploying-protocols/trust" },
              ],
            },
            {
              label: "Transactions and Versions",
              collapsed: true,
              items: [
                { slug: "using-stacks/transactions" },
                { slug: "concepts/program-versioning" },
              ],
            },
            {
              label: "Create Live Data",
              collapsed: true,
              items: [
                { slug: "building-stacks/workflow" },
                { slug: "building-stacks/configuration" },
                { slug: "building-stacks/stack-definitions" },
                { slug: "building-stacks/installation" },
                { slug: "building-stacks/your-first-stack" },
                {
                  label: "Rust DSL",
                  autogenerate: { directory: "building-stacks/rust-dsl" },
                },
                { slug: "cli/idl" },
              ],
            },
            {
              label: "SDKs",
              collapsed: true,
              items: [
                { slug: "using-stacks/installation" },
                { slug: "sdks/typescript" },
                { slug: "sdks/react" },
                { slug: "sdks/react-sessions" },
                { slug: "sdks/rust" },
                { slug: "sdks/python" },
                { slug: "sdks/validation" },
                { slug: "agent-skills/tutorial-ore-dashboard" },
              ],
            },
            {
              label: "Agents and Skills",
              collapsed: true,
              items: [
                { slug: "agent-skills/overview" },
                { slug: "agent-skills/setup-tools" },
                { slug: "agent-skills/setup" },
                { slug: "agent-skills/prompts" },
              ],
            },
            { slug: "getting-started/open-source-and-hosted" },
            {
              label: "Self-host a Stack Server",
              collapsed: true,
              autogenerate: { directory: "a4-server" },
            },
          ],
        },
      ],
      // Enable search when content is ready
      pagefind: true,
    }),
  ],
});
