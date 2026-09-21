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
            "Arete is an agent-first Solana application toolkit. Discover programs and live views, explore on-chain state through MCP, and install typed SDKs for reads, transactions, flows, and real-time data.",
          promote: [
            "getting-started/what-is-arete",
            "using-stacks/quickstart",
            "agent-skills/explore-on-chain",
            "getting-started/from-question-to-app",
            "concepts/programs-views-stacks",
            "using-programs/overview",
            "using-programs/program-reads",
            "using-programs/chain-reads",
            "agent-skills/overview",
            "agent-skills/mcp",
            "sdks/typescript",
            "sdks/react",
          ],
        }),
      ],
      title: "Arete",
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
        EditLink: "./src/components/overrides/EditLink.astro",
        Head: "./src/components/overrides/Head.astro",
        Footer: "./src/components/overrides/Footer.astro",
        MarkdownContent: "./src/components/overrides/MarkdownContent.astro",
        Search: "./src/components/overrides/Search.astro",
        PageTitle: "./src/components/overrides/PageTitle.astro",
      },
      // Autogenerate sidebar from directory structure
      // Contributors only need to add frontmatter to control ordering
      sidebar: [
        {
          label: "Start Here",
          items: [
            { slug: "getting-started/what-is-arete" },
            { slug: "using-stacks/quickstart" },
            { slug: "agent-skills/explore-on-chain" },
            { slug: "getting-started/from-question-to-app" },
          ],
        },
        {
          label: "Core Concepts",
          items: [
            { slug: "concepts/programs-views-stacks" },
            { slug: "using-stacks/how-it-works" },
            { slug: "building-stacks/configuration" },
          ],
        },
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
            { slug: "using-stacks/connect" },
            { slug: "using-stacks/filtering-feeds" },
          ],
        },
        {
          label: "Agents",
          items: [
            { slug: "agent-skills/overview" },
            { slug: "agent-skills/explore" },
            { slug: "agent-skills/mcp" },
            { slug: "agent-skills/prompts" },
            { slug: "agent-skills/setup" },
            { slug: "agent-skills/setup-tools" },
          ],
        },
        {
          label: "Create Live Data",
          items: [
            { slug: "building-stacks/workflow" },
            { slug: "building-stacks/stack-definitions" },
            { slug: "building-stacks/installation" },
            { slug: "building-stacks/your-first-stack" },
            { slug: "building-stacks/finding-idls" },
            {
              label: "Rust DSL",
              items: [
                { slug: "building-stacks/rust-dsl/overview" },
                { slug: "building-stacks/rust-dsl/macros" },
                { slug: "building-stacks/rust-dsl/strategies" },
                { slug: "building-stacks/rust-dsl/resolvers" },
              ],
            },
          ],
        },

        {
          label: "SDK Reference",
          items: [
            {
              label: "TypeScript",
              link: "/sdks/typescript/",
            },
            {
              label: "React",
              link: "/sdks/react/",
            },
            {
              label: "Rust",
              link: "/sdks/rust/",
            },
            {
              label: "Python",
              link: "/sdks/python/",
            },
            {
              label: "Schema Validation",
              link: "/sdks/validation/",
            },
          ],
        },
        {
          label: "CLI",
          autogenerate: { directory: "cli" },
        },
        {
          label: "Self-hosting",
          autogenerate: { directory: "a4-server" },
        },
      ],
      // Enable search when content is ready
      pagefind: true,
    }),
  ],
});
