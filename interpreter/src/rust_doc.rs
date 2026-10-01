//! Doc comments for generated Rust.
//!
//! IDL docs are Markdown lifted from a program's source, and the TypeScript
//! generator writes them into JSDoc as they come. Rust doc comments are
//! linted: clippy's default lints reject Markdown that rustdoc renders well,
//! so generated crates failed `clippy -D warnings` on IDL docs like
//! pancakeswap's, where a list item runs on into an unindented line.
//! [`normalize_doc_lines`] rewrites such Markdown into the form that renders
//! the same and passes those lints, and [`render_doc_comment`] writes it out
//! as `///` lines. Every generator site that writes doc text it did not
//! author goes through them.

/// `docs` (IDL docs or generator notes) as the Markdown lines of a Rust doc
/// comment that rustdoc renders as the IDL wrote it and clippy accepts.
///
/// - A docs entry may hold several lines: it is split at its line breaks
///   (`\n`, `\r\n`, `\r`). Every line is trimmed, as the TypeScript generator
///   trims its JSDoc lines, and tabs become spaces
///   (`clippy::tabs_in_doc_comments`).
/// - Leading and trailing blank lines are dropped, so blank-only docs come
///   back empty and callers can fall back to their own text instead of
///   writing an empty doc comment (`clippy::empty_docs`).
/// - A line Markdown reads as the lazy continuation of a list item's or block
///   quote's paragraph is written out in full: indented to exactly the item's
///   content column and given the quote's `>` markers. Markdown renders both
///   forms the same; clippy rejects the lazy one
///   (`clippy::doc_lazy_continuation`) and indentation past the item's
///   content (`clippy::doc_overindented_list_items`).
/// - A fenced code block passes through as written, except that a fence with
///   no language, or `rust`, is marked `text`: rustdoc would otherwise compile
///   the IDL's snippet as a doctest of the generated crate.
pub(crate) fn normalize_doc_lines(docs: &[String]) -> Vec<String> {
    let lines: Vec<String> = docs
        .iter()
        .flat_map(|doc| {
            doc.replace("\r\n", "\n")
                .split(['\n', '\r'])
                .map(|line| line.trim().replace('\t', "    "))
                .collect::<Vec<_>>()
        })
        .collect();
    let Some(first) = lines.iter().position(|line| !line.is_empty()) else {
        return Vec::new();
    };
    let last = lines
        .iter()
        .rposition(|line| !line.is_empty())
        .unwrap_or(first);

    let mut out = Vec::with_capacity(last + 1 - first);
    // The closing fence (character, minimum length) of the open fenced code
    // block.
    let mut fence: Option<(char, usize)> = None;
    // The open paragraph: its block-quote depth and, when it is a list item's,
    // the item's content column within the quote.
    let mut paragraph: Option<(usize, Option<usize>)> = None;
    for line in lines[first..=last].iter() {
        if let Some((fence_char, fence_len)) = fence {
            if closes_fence(line, fence_char, fence_len) {
                fence = None;
            }
            out.push(line.clone());
            continue;
        }
        if line.is_empty() {
            paragraph = None;
            out.push(String::new());
            continue;
        }

        let (depth, content) = split_block_quote(line);
        if let Some((open_depth, item)) = paragraph {
            // A line that leaves a container open by its markers (fewer `>`
            // than the quote, or no item indentation) is lazy: Markdown then
            // lets any list item, not just a bullet or `1.`, start a block.
            let lazy = depth < open_depth || item.is_some();
            if depth <= open_depth && !interrupts_paragraph(content, lazy) {
                out.push(continuation_line(open_depth, item, content));
                continue;
            }
        }

        let start = block_start(content);
        if let (
            0,
            BlockStart::Fence {
                fence_char,
                fence_len,
                info,
            },
        ) = (depth, &start)
        {
            fence = Some((*fence_char, *fence_len));
            paragraph = None;
            out.push(if info.is_empty() || *info == "rust" {
                format!("{}text", fence_char.to_string().repeat(*fence_len))
            } else {
                line.clone()
            });
            continue;
        }
        paragraph = match start {
            BlockStart::ListItem {
                content_column,
                empty: false,
            } => Some((depth, Some(content_column))),
            BlockStart::Text => Some((depth, None)),
            _ => None,
        };
        out.push(line.clone());
    }
    out
}

/// `lines` as a `///` doc comment at `indent`, one comment line per line
/// (joined with `\n`, no trailing newline). The lines go through
/// [`normalize_doc_lines`] first, which leaves rendered lines as they are.
pub(crate) fn render_doc_comment(lines: &[String], indent: &str) -> String {
    normalize_doc_lines(lines)
        .iter()
        .map(|line| {
            if line.is_empty() {
                format!("{indent}///")
            } else {
                format!("{indent}/// {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// How a (trimmed, quote-free) line starts a Markdown block.
enum BlockStart<'a> {
    /// No content: a blank line inside a block quote.
    Blank,
    /// An opening code fence.
    Fence {
        fence_char: char,
        fence_len: usize,
        info: &'a str,
    },
    /// A list item whose content starts `content_column` columns in (nested
    /// list markers on the same line included).
    ListItem { content_column: usize, empty: bool },
    /// A block that is never a paragraph: a heading, thematic break, HTML
    /// block or footnote definition.
    Other,
    /// Paragraph text.
    Text,
}

fn block_start(content: &str) -> BlockStart<'_> {
    if content.is_empty() {
        return BlockStart::Blank;
    }
    if let Some((fence_char, fence_len, info)) = opening_fence(content) {
        return BlockStart::Fence {
            fence_char,
            fence_len,
            info,
        };
    }
    if is_thematic_break(content)
        || is_atx_heading(content)
        || starts_html_block(content)
        || is_footnote_definition(content)
    {
        return BlockStart::Other;
    }
    if let Some((content_column, empty)) = list_item(content) {
        return BlockStart::ListItem {
            content_column,
            empty,
        };
    }
    BlockStart::Text
}

/// Whether `content` ends an open paragraph instead of continuing it. When
/// the line is `lazy`, any list item starts a block; otherwise only a bullet
/// or an item numbered 1, and only with content (CommonMark's rule).
fn interrupts_paragraph(content: &str, lazy: bool) -> bool {
    match block_start(content) {
        BlockStart::Text => false,
        BlockStart::ListItem { empty, .. } if !lazy => {
            !empty && (is_bullet_item(content) || ordered_number(content) == Some(1))
        }
        _ => true,
    }
}

/// A paragraph continuation line with its quote markers and item indentation
/// written out.
fn continuation_line(depth: usize, item: Option<usize>, content: &str) -> String {
    let mut line = String::new();
    if depth > 0 {
        line.push_str(&">".repeat(depth));
        line.push(' ');
    }
    line.push_str(&" ".repeat(item.unwrap_or(0)));
    line.push_str(content);
    line
}

/// The number of leading block-quote markers (`>`, optionally spaced) and
/// the trimmed rest of the line.
fn split_block_quote(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line;
    while let Some(after) = rest.strip_prefix('>') {
        depth += 1;
        rest = after.trim_start();
    }
    (depth, rest)
}

/// An opening code fence: its character, length and info string.
fn opening_fence(content: &str) -> Option<(char, usize, &str)> {
    let fence_char = content.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let fence_len = content.chars().take_while(|c| *c == fence_char).count();
    if fence_len < 3 {
        return None;
    }
    let info = content[fence_len..].trim();
    if fence_char == '`' && info.contains('`') {
        return None;
    }
    Some((fence_char, fence_len, info))
}

fn closes_fence(line: &str, fence_char: char, fence_len: usize) -> bool {
    line.len() >= fence_len && line.chars().all(|c| c == fence_char)
}

fn is_thematic_break(content: &str) -> bool {
    let Some(marker) = content
        .chars()
        .next()
        .filter(|c| matches!(c, '*' | '-' | '_'))
    else {
        return false;
    };
    content.chars().all(|c| c == marker || c == ' ') && content.matches(marker).count() >= 3
}

fn is_atx_heading(content: &str) -> bool {
    let hashes = content.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes)
        && content[hashes..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ' || c == '\t')
}

/// HTML blocks that can start inside a paragraph (CommonMark types 1-6).
fn starts_html_block(content: &str) -> bool {
    const RAW_TAGS: [&str; 4] = ["pre", "script", "style", "textarea"];
    const BLOCK_TAGS: [&str; 62] = [
        "address",
        "article",
        "aside",
        "base",
        "basefont",
        "blockquote",
        "body",
        "caption",
        "center",
        "col",
        "colgroup",
        "dd",
        "details",
        "dialog",
        "dir",
        "div",
        "dl",
        "dt",
        "fieldset",
        "figcaption",
        "figure",
        "footer",
        "form",
        "frame",
        "frameset",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "head",
        "header",
        "hr",
        "html",
        "iframe",
        "legend",
        "li",
        "link",
        "main",
        "menu",
        "menuitem",
        "nav",
        "noframes",
        "ol",
        "optgroup",
        "option",
        "p",
        "param",
        "search",
        "section",
        "summary",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "title",
        "tr",
        "track",
        "ul",
    ];
    let Some(rest) = content.strip_prefix('<') else {
        return false;
    };
    if rest.starts_with("!--") || rest.starts_with('?') || rest.starts_with("![CDATA[") {
        return true;
    }
    if rest
        .strip_prefix('!')
        .and_then(|after| after.chars().next())
        .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return true;
    }
    let (closing, tag) = match rest.strip_prefix('/') {
        Some(tag) => (true, tag),
        None => (false, rest),
    };
    let name_len = tag
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .count();
    let name = tag[..name_len].to_ascii_lowercase();
    let after = &tag[name_len..];
    let ends_name = after.is_empty() || after.starts_with([' ', '>']);
    (!closing && RAW_TAGS.contains(&name.as_str()) && ends_name)
        || (BLOCK_TAGS.contains(&name.as_str()) && (ends_name || after.starts_with("/>")))
}

fn is_footnote_definition(content: &str) -> bool {
    content
        .strip_prefix("[^")
        .and_then(|rest| rest.find("]:"))
        .is_some_and(|end| end > 0)
}

fn is_bullet_item(content: &str) -> bool {
    content.starts_with(['-', '+', '*'])
}

/// The number of an ordered list item.
fn ordered_number(content: &str) -> Option<u64> {
    let digits = content.chars().take_while(|c| c.is_ascii_digit()).count();
    content[..digits].parse().ok()
}

/// A list item's content column (nested list markers on the same line
/// included) and whether it has no content. Mirrors CommonMark: a bullet
/// (`-`, `+`, `*`) or up to nine digits and `.` or `)`, then 1-4 spaces (more
/// is one space plus indented content) or the end of the line.
fn list_item(content: &str) -> Option<(usize, bool)> {
    let marker = if is_bullet_item(content) {
        1
    } else {
        let digits = content.chars().take_while(|c| c.is_ascii_digit()).count();
        if !(1..=9).contains(&digits) || !content[digits..].starts_with(['.', ')']) {
            return None;
        }
        digits + 1
    };
    let rest = &content[marker..];
    if rest.is_empty() {
        return Some((marker + 1, true));
    }
    let spaces = rest.chars().take_while(|c| *c == ' ').count();
    if spaces == 0 {
        return None;
    }
    let item_content = rest.trim_start();
    if item_content.is_empty() {
        return Some((marker + 1, true));
    }
    let column = marker + if spaces > 4 { 1 } else { spaces };
    if !is_thematic_break(item_content) {
        if let Some((nested, empty)) = list_item(item_content) {
            return Some((column + nested, empty));
        }
    }
    Some((column, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    fn markdown(text: &[&str]) -> Vec<String> {
        normalize_doc_lines(&lines(text))
    }

    #[test]
    fn indents_lazy_list_item_continuations_to_the_item_content() {
        // pancakeswap amm_v3 `create_pool`.
        assert_eq!(
            markdown(&[
                "Creates a pool for the given token pair and the initial price",
                "",
                "# Arguments",
                "",
                "* `ctx`- The context of accounts",
                "* `sqrt_price_x64` - the initial sqrt price of the pool as a Q64.64",
                "Note: The open_time must be smaller than the current block_timestamp on chain.",
            ]),
            lines(&[
                "Creates a pool for the given token pair and the initial price",
                "",
                "# Arguments",
                "",
                "* `ctx`- The context of accounts",
                "* `sqrt_price_x64` - the initial sqrt price of the pool as a Q64.64",
                "  Note: The open_time must be smaller than the current block_timestamp on chain.",
            ])
        );
        // Ordered items, wide markers, extra marker spacing and nested markers.
        assert_eq!(
            markdown(&[
                "1. first",
                "runs on",
                "10. tenth",
                "runs on",
                "*   spaced",
                "runs on"
            ]),
            lines(&[
                "1. first",
                "   runs on",
                "10. tenth",
                "    runs on",
                "*   spaced",
                "    runs on"
            ])
        );
        assert_eq!(
            markdown(&["- - nested", "runs on"]),
            lines(&["- - nested", "    runs on"])
        );
        // Every continuation line, not just the first.
        assert_eq!(
            markdown(&["- item", "one", "two"]),
            lines(&["- item", "  one", "  two"])
        );
    }

    #[test]
    fn leaves_blocks_that_end_a_list_item_alone() {
        // New items (any number once lazy), headings, breaks, fences, quotes,
        // HTML blocks and a blank line all end the item's paragraph.
        for line in [
            "- next",
            "+ next",
            "2. next",
            "1) next",
            "# Heading",
            "---",
            "* * *",
            "> quote",
            "<div>",
            "<!-- note -->",
            "[^1]: footnote",
        ] {
            let rendered = markdown(&["- item", line]);
            assert_eq!(rendered[1], line, "{line:?}");
        }
        assert_eq!(
            markdown(&["- item", "", "Paragraph."]),
            lines(&["- item", "", "Paragraph."])
        );
        // Outside a list, a non-1 number cannot start one: the line is text.
        assert_eq!(
            markdown(&["Text", "2. continues", "the paragraph"]),
            lines(&["Text", "2. continues", "the paragraph"])
        );
        // An empty item holds no paragraph to continue.
        assert_eq!(markdown(&["-", "Text"]), lines(&["-", "Text"]));
    }

    #[test]
    fn writes_out_lazy_block_quote_markers() {
        assert_eq!(
            markdown(&[
                "> quoted",
                "runs on",
                ">> nested",
                "> runs on",
                "> - item",
                "> runs on"
            ]),
            lines(&[
                "> quoted",
                "> runs on",
                ">> nested",
                ">> runs on",
                "> - item",
                ">   runs on"
            ])
        );
    }

    #[test]
    fn splits_trims_and_drops_blank_edges() {
        // pyth_solana_receiver `VerificationLevel`: one entry, several lines.
        assert_eq!(
            markdown(&[
                "",
                "* This enum represents how many guardian signatures were checked\n * If full, all of them\r\n\tand more",
                "   ",
            ]),
            lines(&[
                "* This enum represents how many guardian signatures were checked",
                "* If full, all of them",
                "  and more",
            ])
        );
        assert_eq!(markdown(&["tab\tinside"]), lines(&["tab    inside"]));
        assert!(markdown(&["", "  ", "\n"]).is_empty());
        assert!(markdown(&[]).is_empty());
    }

    #[test]
    fn marks_untyped_fences_as_text_and_keeps_their_contents() {
        assert_eq!(
            markdown(&[
                "- item", "```", "- a", "b", "```", "rust:", "~~~rust", "x", "~~~", "```json",
                "{}", "```"
            ]),
            lines(&[
                "- item", "```text", "- a", "b", "```", "rust:", "~~~text", "x", "~~~", "```json",
                "{}", "```"
            ])
        );
    }

    #[test]
    fn rendering_is_idempotent() {
        let docs = lines(&[
            "- item",
            "runs on",
            "> quote",
            "lazy",
            "> - quoted item",
            "lazy",
            "```",
            "code",
            "```",
        ]);
        let once = normalize_doc_lines(&docs);
        assert_eq!(normalize_doc_lines(&once), once);
        assert_eq!(
            render_doc_comment(&once, "    "),
            render_doc_comment(&docs, "    ")
        );
    }

    #[test]
    fn renders_doc_comment_lines() {
        assert_eq!(
            render_doc_comment(&lines(&["Summary.", "", "- item", "more"]), "    "),
            "    /// Summary.\n    ///\n    /// - item\n    ///   more"
        );
        assert_eq!(render_doc_comment(&lines(&["", " "]), ""), "");
    }

    /// Runs clippy with `-D warnings` over a crate whose doc comments are
    /// rendered from IDL docs clippy rejects as written: the catalog's
    /// patterns (pancakeswap, bubblegum, pyth_solana_receiver) and the
    /// Markdown cases above.
    #[test]
    fn rendered_doc_comments_pass_clippy() {
        let cases: Vec<Vec<&str>> = vec![
            vec![
                "Creates a pool for the given token pair and the initial price",
                "",
                "# Arguments",
                "",
                "* `ctx`- The context of accounts",
                "* `sqrt_price_x64` - the initial sqrt price of the pool as a Q64.64",
                "Note: The open_time must be smaller than the current block_timestamp on chain.",
            ],
            vec![
                "Mints a new asset.",
                "1. Uses the streamlined `MetadataV2` arguments, which eliminate the verified",
                "flag.  In `MetadataV2`, any collection included is automatically considered",
                "verified.",
                "2. Allows for freezing/thawing of the asset, as well as setting it to be",
                "permanently non-transferable (soulbound).",
            ],
            vec!["* This enum represents\n * If full, quorum\nwas checked"],
            vec![
                "1. first",
                "runs on",
                "10. tenth",
                "runs on",
                "*   spaced",
                "runs on",
            ],
            vec!["- - nested", "runs on", "- next", "2. lazy item", "runs on"],
            vec![
                "> quoted",
                "runs on",
                ">> nested",
                "> runs on",
                "> - item",
                "> runs on",
            ],
            vec!["- item", "```", "not rust", "```", "after"],
            vec!["tab\tinside", "- item", "\ttabbed continuation"],
            vec!["Text", "2. continues", "the paragraph", "-", "Text"],
        ];
        let mut source = String::from("//! Rendered doc comments.\n");
        for (index, case) in cases.iter().enumerate() {
            source.push_str(&render_doc_comment(&lines(case), ""));
            source.push_str(&format!("\npub fn case_{index}() {{}}\n"));
        }

        let base = std::env::temp_dir().join(format!(
            "arete-rust-doc-clippy-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src")).expect("create probe crate");
        std::fs::write(
            base.join("Cargo.toml"),
            "[package]\nname = \"rendered-doc-comments\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .expect("write probe manifest");
        std::fs::write(base.join("src/lib.rs"), &source).expect("write probe source");

        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let linted = Command::new(cargo)
            .args(["clippy", "--quiet", "--offline", "--manifest-path"])
            .arg(base.join("Cargo.toml"))
            .args(["--", "-D", "warnings"])
            .env("CARGO_TARGET_DIR", base.join("target"))
            .output()
            .expect("cargo clippy must be available for the doc comment check");
        let _ = std::fs::remove_dir_all(&base);
        assert!(
            linted.status.success(),
            "clippy rejected rendered doc comments:\n{source}\nstderr:\n{}",
            String::from_utf8_lossy(&linted.stderr),
        );
    }
}
