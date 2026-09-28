export interface SearchablePage {
  slug: string;
  title: string;
  content: string;
}

export interface DocsSection {
  heading: string;
  text: string;
}

export interface DocsSearchIndex {
  readonly documents: ReadonlyArray<unknown>;
  readonly documentFrequency: ReadonlyMap<string, number>;
  readonly averageLength: number;
}

export interface DocsSearchResult {
  slug: string;
  title: string;
  /** Heading of the section the snippet was taken from ("" before any heading). */
  section: string;
  snippet: string;
  score: number;
  matchedTerms: number;
  totalTerms: number;
}

export function queryTerms(query: string): string[];
export function documentTerms(text: string): string[];
export function sections(content: string): DocsSection[];
export function stripPageDirective(content: string): string;
export function buildSearchIndex(
  pages: ReadonlyArray<SearchablePage>,
): DocsSearchIndex;
export function searchDocs(
  index: DocsSearchIndex,
  query: string,
  limit?: number,
): DocsSearchResult[];
