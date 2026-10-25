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
            "Arete is a platform for building and running Solana protocols. Compose programs into workflows, develop in persistent mainnet forks, and integrate generated clients into applications and agents.",
          promote: [
            "getting-started/what-is-arete",
            "using-stacks/quickstart",
            "concepts/protocols",
            "building-protocols/definition",
            "building-protocols/lifecycle",
            "cli/local-development",
            "development/application",
            "integrating-protocols/overview",
            "integrating-protocols/execution",
            "integrating-protocols/react",
            "sdks/react-sessions",
            "automations/overview",
            "deploying-protocols/overview",
            "deploying-protocols/trust",
            "agent-skills/protocols",
            "reference/protocols",
            "using-programs/overview",
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
      // Keep existing slugs stable; navigation follows the protocol workflow.
      sidebar: [
        {
          label: "Start Here",
          items: [
            { label: "Get started", link: "/" },
            { slug: "getting-started/what-is-arete" },
            { slug: "using-stacks/quickstart" },
            { slug: "integrating-protocols/overview" },
            { slug: "concepts/protocols" },
            { slug: "getting-started/from-question-to-app" },
            { slug: "getting-started/open-source-and-hosted" },
          ],
        },
        {
          label: "Build Protocols",
          items: [
            { slug: "building-protocols/definition" },
            { slug: "building-protocols/lifecycle" },
            { slug: "building-protocols/handlers" },
            { slug: "building-protocols/automation" },
            { slug: "building-protocols/squads" },
            { slug: "building-protocols/tutorial-lifecycle" },
          ],
        },
        {
          label: "Automate for Users",
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
          items: [
            { slug: "cli/local-development" },
            { slug: "development/application" },
            { slug: "development/testing" },
          ],
        },
        {
          label: "Integrate Protocols",
          items: [
            { slug: "integrating-protocols/execution" },
            { slug: "integrating-protocols/react" },
            { slug: "integrating-protocols/events" },
            { slug: "agent-skills/protocols" },
          ],
        },
        {
          label: "Deploy and Publish",
          items: [
            { slug: "deploying-protocols/overview" },
            { slug: "deploying-protocols/trust" },
          ],
        },
        {
          label: "Programs and Live Data",
          items: [
            { slug: "concepts/programs-views-stacks" },
            { slug: "concepts/program-versioning" },
            {
              label: "Programs",
              items: [
                { slug: "using-programs/overview" },
                { slug: "using-programs/program-reads" },
                { slug: "using-programs/chain-reads" },
                { slug: "using-stacks/transactions" },
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
              label: "Create Live Data",
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
              ],
            },
          ],
        },
        {
          label: "Reference",
          items: [
            { slug: "reference/protocols" },
            {
              label: "SDKs",
              items: [
                { slug: "using-stacks/installation" },
                { slug: "sdks/typescript" },
                { slug: "sdks/react" },
                { slug: "sdks/react-sessions" },
                { slug: "sdks/rust" },
                { slug: "sdks/python" },
                { slug: "sdks/validation" },
              ],
            },
            {
              label: "Agents and Discovery",
              items: [
                { slug: "agent-skills/overview" },
                { slug: "agent-skills/explore" },
                { slug: "agent-skills/explore-on-chain" },
                { slug: "agent-skills/mcp" },
                { slug: "agent-skills/prompts" },
                { slug: "agent-skills/setup" },
                { slug: "agent-skills/setup-tools" },
                { slug: "agent-skills/tutorial-ore-dashboard" },
              ],
            },
            { slug: "cli/commands" },
            { slug: "cli/idl" },
            { slug: "reference/api-problems" },
            {
              label: "Self-host a Stack Server",
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
