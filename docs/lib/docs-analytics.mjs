/**
 * Docs-site analytics helpers. No project keys live here.
 *
 * Custom events go through `trackDocsEvent`, which waits for the PostHog stub
 * created by /posthog-init.js so a click during init is not dropped.
 *
 * UTM convention for measurable links to arete.run or docs.arete.run:
 * - X posts: `xPostHref`. utm_source=x, utm_medium=social,
 *   utm_campaign=<week-id> (for example 2026-w44),
 *   utm_content=<slug>_<account> (for example quickstart_usearete).
 * - CTAs on this docs site: `docsCtaHref`, written as an ordinary Markdown
 *   link (not a JSX component). stripMdx drops PascalCase tags, so a
 *   component would disappear from Copy Markdown and the .md twins.
 *   utm_source=docs, utm_medium=referral, utm_campaign=docs,
 *   utm_content=<page>_<target> (for example about_home).
 */

const UTM_KEYS = [
  "utm_source",
  "utm_medium",
  "utm_campaign",
  "utm_content",
  "utm_term",
];

const SDK_TAB_LANGUAGES = [
  ["typescript", /\btypescript\b/i],
  ["javascript", /\bjavascript\b/i],
  ["python", /\bpython\b/i],
  ["rust", /\brust\b/i],
  ["react", /\breact\b/i],
];

const SDK_PATH_LANGUAGES = {
  typescript: "typescript",
  react: "react",
  "react-sessions": "react",
  rust: "rust",
  python: "python",
};

export function withUtm(href, utm) {
  const url = new URL(href, "https://docs.arete.run");
  url.searchParams.set("utm_source", utm.source);
  url.searchParams.set("utm_medium", utm.medium);
  url.searchParams.set("utm_campaign", utm.campaign);
  url.searchParams.set("utm_content", utm.content);
  if (href.startsWith("/") && !href.startsWith("//")) {
    return `${url.pathname}${url.search}${url.hash}`;
  }
  return url.toString();
}

/** Link used from an X post. `campaign` is a week id such as `2026-w44`. */
export function xPostHref(href, { campaign, slug, account }) {
  return withUtm(href, {
    source: "x",
    medium: "social",
    campaign,
    content: `${slug}_${account}`,
  });
}

/** Docs-site CTA to the marketing site or another Arete origin. */
export function docsCtaHref(href, content) {
  return withUtm(href, {
    source: "docs",
    medium: "referral",
    campaign: "docs",
    content,
  });
}

export function currentUtmParams(search) {
  const params = new URLSearchParams(search);
  const utm = {};
  for (const key of UTM_KEYS) {
    const value = params.get(key);
    if (value) utm[key] = value;
  }
  return utm;
}

export function outboundDestination(href) {
  let url;
  try {
    url = new URL(href, "https://docs.arete.run");
  } catch {
    return null;
  }
  const host = url.hostname.replace(/^www\./, "");
  if (host === "github.com") return "github";
  if (host === "x.com" || host === "twitter.com") return "x";
  return null;
}

export function sdkLanguageFromTabLabel(label) {
  const text = label.trim();
  if (!text) return null;
  for (const [language, pattern] of SDK_TAB_LANGUAGES) {
    if (pattern.test(text)) return language;
  }
  return null;
}

export function sdkLanguageFromPath(pathname) {
  const match = pathname.match(/\/sdks\/([^/]+)/);
  if (!match) return null;
  return SDK_PATH_LANGUAGES[match[1]] ?? null;
}

/**
 * Classify a copied snippet. Returns null when it is not an install or CLI
 * command. Expressive Code stores newlines in `data-code` as U+007F.
 */
export function classifyCopiedSnippet(code) {
  const text = code.replace(/\u007f/g, "\n").trim();
  if (!text) return null;
  const preview = (text.split("\n").find((line) => line.trim()) ?? "").slice(
    0,
    80,
  );

  // curl/wget/irm count only when they fetch the installer, not any HTTP call.
  if (/\b(curl|wget|irm)\b[\s\S]{0,400}\binstall\.(sh|ps1)\b/.test(text)) {
    return { snippet_kind: "install_script", code_preview: preview };
  }
  if (/(^|\n)\s*(a4|npx\s+@usearete\/a4)\b/.test(text)) {
    return { snippet_kind: "cli", code_preview: preview };
  }
  if (
    /\b(npm|pnpm|yarn|bun)\s+(install|add)\b/.test(text) ||
    /\b(pip3?|uv\s+pip)\s+install\b/.test(text) ||
    /\bpython3?\s+-m\s+pip\s+install\b/.test(text) ||
    /\bpoetry\s+add\b/.test(text) ||
    /\bcargo\s+install\b/.test(text)
  ) {
    return { snippet_kind: "package_install", code_preview: preview };
  }
  return null;
}

export function codePreview(code) {
  const text = code.replace(/\u007f/g, "\n");
  return (text.split("\n").find((line) => line.trim()) ?? "").slice(0, 80);
}

/**
 * Capture a docs event once PostHog's stub exists. Safe to call from several
 * bundled scripts: the queue lives on window.
 */
export function trackDocsEvent(eventName, properties = {}) {
  if (typeof window === "undefined") return;
  const payload = {
    eventName,
    properties: {
      page: window.location.pathname,
      ...currentUtmParams(window.location.search),
      ...properties,
    },
  };
  const posthog = window.posthog;
  if (posthog && typeof posthog.capture === "function") {
    posthog.capture(payload.eventName, payload.properties);
    return;
  }

  const queue = window.__areteDocsEventQueue || [];
  queue.push(payload);
  window.__areteDocsEventQueue = queue;
  if (window.__areteDocsEventFlush) return;
  window.__areteDocsEventFlush = true;

  const started = Date.now();
  const timer = window.setInterval(() => {
    const ph = window.posthog;
    if (ph && typeof ph.capture === "function") {
      const pending = window.__areteDocsEventQueue || [];
      window.__areteDocsEventQueue = [];
      for (const item of pending) {
        ph.capture(item.eventName, item.properties);
      }
      window.clearInterval(timer);
      window.__areteDocsEventFlush = false;
      return;
    }
    if (Date.now() - started > 10000) {
      window.clearInterval(timer);
      window.__areteDocsEventFlush = false;
    }
  }, 250);
}
