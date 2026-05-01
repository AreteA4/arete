// Strips MDX-only constructs (ESM imports/exports, JSX tags) from a body so
// the result is plain CommonMark suitable for non-MDX consumers (LLM agents,
// search indexers). Code fences are preserved verbatim — JSX-like syntax
// inside fenced code is left alone.
//
// Limitations: regex-based, not an MDX AST parser. Handles the patterns used
// in the Arete docs (single- and multi-line component tags with prop blocks
// like `steps={[...]}`) but may miss exotic JSX where attribute values
// contain a literal `>`. Upgrade to remark-mdx if that becomes a real case.

export function stripMdx(source: string): string {
  // Split on fenced code blocks so we can leave them untouched.
  const parts = source.split(/(```[\s\S]*?```)/g);
  return parts
    .map((part, i) => {
      if (i % 2 === 1) return part; // a code fence
      return (
        part
          // ESM import/export lines (single-line; multi-line imports are uncommon)
          .replace(/^\s*(import|export)\s.+;?\s*$/gm, "")
          // Self-closing JSX <Foo ... /> (multi-line via [^>]*)
          .replace(/<[A-Z][A-Za-z0-9]*(\s[^>]*?)?\/>/g, "")
          // Opening JSX <Foo ...>
          .replace(/<[A-Z][A-Za-z0-9]*(\s[^>]*)?>/g, "")
          // Closing JSX </Foo>
          .replace(/<\/[A-Z][A-Za-z0-9]*>/g, "")
          // Collapse runs of blank lines left behind
          .replace(/\n{3,}/g, "\n\n")
      );
    })
    .join("")
    .trim();
}
