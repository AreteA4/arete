/** Term-based ranking for the documentation MCP server's search_docs tool.
 *
 *  Pages are scored with BM25 over their text plus separate bonuses for query
 *  terms in the title, the slug and the best-matching heading, so a query
 *  matches on its words rather than as one exact phrase. Identifiers such as
 *  `AreteProvider` index both whole and by their parts, and the snippet comes
 *  from the heading section with the most query terms.
 */

const STOPWORDS = new Set([
  "a",
  "an",
  "and",
  "are",
  "as",
  "at",
  "be",
  "by",
  "can",
  "do",
  "does",
  "for",
  "from",
  "how",
  "i",
  "in",
  "is",
  "it",
  "my",
  "of",
  "on",
  "or",
  "the",
  "this",
  "to",
  "use",
  "using",
  "what",
  "when",
  "where",
  "which",
  "with",
  "you",
  "your",
]);

// Body term frequency (headings count as body text too) is saturated by BM25.
// Title, slug and heading matches add IDF-weighted bonuses outside that
// saturation so a page named for the query outranks one that repeats a word.
const TITLE_BONUS = 2;
const SLUG_BONUS = 1.5;
const HEADING_BONUS = 1.5;

const K1 = 1.2;
// Milder than the usual 0.75: short legal/about pages shouldn't outrank
// reference pages on incidental words.
const B = 0.5;

const SNIPPET_BEFORE = 150;
const SNIPPET_AFTER = 450;
const SNIPPET_FALLBACK = 600;

// Light stemming shared by queries and documents: plurals, then one common
// suffix (installation/installed → install, streaming → stream).
function normalize(token) {
  let term = token;
  if (term.length > 4 && term.endsWith("ies")) {
    term = `${term.slice(0, -3)}y`;
  } else if (
    term.length > 3 &&
    term.endsWith("s") &&
    !/(ss|us|is)$/.test(term)
  ) {
    term = term.slice(0, -1);
  }
  if (term.length > 7 && term.endsWith("ation")) return term.slice(0, -5);
  if (term.length > 6 && term.endsWith("ing")) return term.slice(0, -3);
  if (term.length > 5 && term.endsWith("ed") && !term.endsWith("eed")) {
    return term.slice(0, -2);
  }
  return term;
}

function words(text) {
  return text.match(/[A-Za-z0-9_]+/g) ?? [];
}

// Splits `deployWithCheckpoint` into deploy/with/checkpoint and
// `HTTPServer` into http/server.
function identifierParts(word) {
  return word
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
    .split(/[\s_]+/)
    .filter(Boolean);
}

/** Query terms: whole words, lower-cased and singularised, without stopwords.
 *  Identifier parts are not added, so `AreteProvider` stays one precise term. */
export function queryTerms(query) {
  const seen = new Set();
  const terms = [];
  for (const word of words(query)) {
    const lower = word.toLowerCase();
    // Check before stemming too, or "does" would survive as "doe".
    if (STOPWORDS.has(lower)) continue;
    const term = normalize(lower);
    if (STOPWORDS.has(term) || seen.has(term)) continue;
    seen.add(term);
    terms.push(term);
  }
  return terms;
}

/** Document terms: whole words plus the parts of camelCase and snake_case
 *  identifiers, so `arete provider` also finds `AreteProvider`. */
export function documentTerms(text) {
  const terms = [];
  for (const word of words(text)) {
    const whole = normalize(word.toLowerCase());
    terms.push(whole);
    const parts = identifierParts(word);
    if (parts.length > 1) {
      for (const part of parts) {
        const term = normalize(part.toLowerCase());
        if (term !== whole) terms.push(term);
      }
    }
  }
  return terms;
}

// Every per-page markdown twin starts with the same agent directive pointing
// at llms.txt (src/lib/markdown-page.ts). It is navigation, not content, and
// would otherwise match common words on every page.
const PAGE_DIRECTIVE = /^\s*> For the complete documentation index\b[^\n]*(?:\n|$)/;

/** Page content without the leading agent directive blockquote. */
export function stripPageDirective(content) {
  return content.replace(PAGE_DIRECTIVE, "").replace(/^\s+/, "");
}

function addTerms(frequencies, terms) {
  for (const term of terms) {
    frequencies.set(term, (frequencies.get(term) ?? 0) + 1);
  }
}

/** Splits markdown into heading sections. Text before the first heading is a
 *  section with an empty heading. Fenced code is kept with its section and is
 *  never mistaken for a heading. */
export function sections(content) {
  const result = [];
  let heading = "";
  let lines = [];
  let fenced = false;
  const flush = () => {
    const text = lines.join("\n").trim();
    if (heading || text) result.push({ heading, text });
  };
  for (const line of content.split("\n")) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    const match = fenced ? null : /^#{1,6}\s+(.+?)\s*#*\s*$/.exec(line);
    if (match) {
      flush();
      heading = match[1];
      lines = [];
    } else {
      lines.push(line);
    }
  }
  flush();
  return result;
}

/** Precomputes term statistics for a list of `{ slug, title, content }`
 *  pages. Build once per loaded docs index and reuse for every query. */
export function buildSearchIndex(pages) {
  const documentFrequency = new Map();
  const documents = pages.map((page) => {
    const content = stripPageDirective(page.content);
    const pageSections = sections(content);
    const frequencies = new Map();
    const titleTerms = new Set(documentTerms(page.title));
    const slugTerms = new Set(documentTerms(page.slug.replace(/[/-]/g, " ")));
    const headingTerms = [];
    for (const section of pageSections) {
      const terms = documentTerms(section.heading);
      headingTerms.push(new Set(terms));
      addTerms(frequencies, terms);
      addTerms(frequencies, documentTerms(section.text));
    }
    let length = 0;
    for (const value of frequencies.values()) length += value;
    const allTerms = new Set([
      ...frequencies.keys(),
      ...titleTerms,
      ...slugTerms,
    ]);
    for (const term of allTerms) {
      documentFrequency.set(term, (documentFrequency.get(term) ?? 0) + 1);
    }
    return {
      page,
      content,
      sections: pageSections,
      frequencies,
      length,
      titleTerms,
      // Whole title words, for "the query names this page".
      titleWords: queryTerms(page.title),
      slugTerms,
      headingTerms,
    };
  });
  const totalLength = documents.reduce((sum, doc) => sum + doc.length, 0);
  return {
    documents,
    documentFrequency,
    averageLength: documents.length ? totalLength / documents.length : 0,
  };
}

function inverseDocumentFrequency(index, term) {
  const n = index.documents.length;
  const df = index.documentFrequency.get(term) ?? 0;
  return Math.log(1 + (n - df + 0.5) / (df + 0.5));
}

function sectionTermCount(section, terms) {
  const present = new Set(documentTerms(`${section.heading}\n${section.text}`));
  return terms.filter((term) => present.has(term)).length;
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function firstTermOffset(text, terms) {
  let best = -1;
  for (const term of terms) {
    const match = new RegExp(`\\b${escapeRegex(term)}`, "i").exec(text);
    if (match && (best < 0 || match.index < best)) best = match.index;
  }
  return best;
}

function snippetFor(document, terms) {
  let best = null;
  let bestCount = 0;
  for (const section of document.sections) {
    const count = sectionTermCount(section, terms);
    if (count > bestCount) {
      best = section;
      bestCount = count;
    }
  }
  if (!best) {
    return {
      section: "",
      snippet: document.content.slice(0, SNIPPET_FALLBACK).trim(),
    };
  }
  const offset = firstTermOffset(best.text, terms);
  if (offset < 0) {
    return {
      section: best.heading,
      snippet: best.text.slice(0, SNIPPET_FALLBACK).trim(),
    };
  }
  const start = Math.max(0, offset - SNIPPET_BEFORE);
  const end = Math.min(best.text.length, offset + SNIPPET_AFTER);
  const prefix = start > 0 ? "…" : "";
  const suffix = end < best.text.length ? "…" : "";
  return {
    section: best.heading,
    snippet: prefix + best.text.slice(start, end).trim() + suffix,
  };
}

/** Ranks pages for `query`. Every returned page matches at least one query
 *  term; pages matching more of the terms, in titles, slugs or headings, or
 *  containing the whole query as a phrase rank higher. */
export function searchDocs(index, query, limit = 5) {
  const terms = queryTerms(query);
  if (terms.length === 0) return [];
  const phrase = query.trim().toLowerCase();
  const multiWord = terms.length > 1;
  const scored = [];
  const idf = new Map(
    terms.map((term) => [term, inverseDocumentFrequency(index, term)]),
  );
  for (const document of index.documents) {
    let score = 0;
    let matched = 0;
    const norm = 1 - B + B * (document.length / (index.averageLength || 1));
    for (const term of terms) {
      const tf = document.frequencies.get(term) ?? 0;
      const inTitle = document.titleTerms.has(term);
      const inSlug = document.slugTerms.has(term);
      if (!tf && !inTitle && !inSlug) continue;
      matched += 1;
      const weight = idf.get(term);
      if (tf) score += weight * ((tf * (K1 + 1)) / (tf + K1 * norm));
      if (inTitle) score += weight * TITLE_BONUS;
      if (inSlug) score += weight * SLUG_BONUS;
    }
    if (matched === 0) continue;
    // The single heading covering the most query weight, e.g. `a4 stack compose`.
    let bestHeading = 0;
    for (const heading of document.headingTerms) {
      let weight = 0;
      for (const term of terms) if (heading.has(term)) weight += idf.get(term);
      if (weight > bestHeading) bestHeading = weight;
    }
    score += bestHeading * HEADING_BONUS;
    // The query names this page, e.g. "wallet adapter TypeScript SDK" and the
    // page titled "TypeScript SDK".
    if (
      document.titleWords.length > 0 &&
      document.titleWords.every((term) => idf.has(term))
    ) {
      score *= 1.5;
    }
    // Pages covering more of the query outrank pages that repeat one term.
    score *= 0.5 + matched / terms.length;
    if (multiWord && document.page.title.toLowerCase().includes(phrase)) {
      score *= 2;
    } else if (
      multiWord &&
      document.content.toLowerCase().includes(phrase)
    ) {
      score *= 1.5;
    }
    scored.push({ document, score, matched });
  }
  scored.sort(
    (a, b) =>
      b.score - a.score ||
      a.document.page.slug.localeCompare(b.document.page.slug),
  );
  return scored.slice(0, limit).map(({ document, score, matched }) => {
    const { section, snippet } = snippetFor(document, terms);
    return {
      slug: document.page.slug,
      title: document.page.title,
      section,
      snippet,
      score,
      matchedTerms: matched,
      totalTerms: terms.length,
    };
  });
}
