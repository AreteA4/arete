//! The functions a TypeScript SDK extension adds, read from its source for
//! the SDK reference: the `read.*` helpers its `createRead` returns and the
//! operations its `createOperations` returns, each with its parameters, its
//! JSDoc `@title` and description, and, for a read, the shape it resolves
//! to when its `return` statements say.
//!
//! Nothing is executed or type-checked. The source is scanned with string
//! literals and comments blanked, so brackets inside them are not code; a
//! function whose declaration cannot be found is still listed, without
//! parameters. These sources come from the SDK generator and the published
//! extensions it stages, which keep to plain declarations.

use arete_mcp::sdk_reference::FunctionReference;
use regex::Regex;

/// The most functions listed from one extension.
const MAX_FUNCTIONS: usize = 256;

/// Deepest namespace nesting followed in a returned object.
const MAX_NESTING: usize = 6;

/// The namespaces of plain helpers an extension definition may declare.
const HELPER_NAMESPACES: [&str; 4] = ["addresses", "math", "constants", "defaults"];

/// The `read.*` helpers `source` adds: the object its `createRead` returns,
/// else the names its `readArgCounts` declares.
pub(super) fn reads(source: &str) -> Vec<FunctionReference> {
    let outline = Outline::new(source);
    let mut leaves = outline.returned_leaves("createRead");
    if leaves.is_empty() {
        leaves = outline.read_arg_count_names();
    }
    leaves
        .into_iter()
        .map(|leaf| outline.function(&format!("read.{}", leaf.path), &leaf, true))
        .collect()
}

/// The helpers `source`'s extension definition declares: `addresses.var`,
/// `math.finalValue`, or a namespace alone (`addresses`) when it is not
/// written in place.
pub(super) fn helpers(source: &str) -> Vec<String> {
    Outline::new(source).helpers()
}

/// The operations `source` adds: the object its `createOperations` returns,
/// grouped as `instructions.*`, `transactions.*` and `flows.*`.
pub(super) fn operations(source: &str) -> Vec<FunctionReference> {
    let outline = Outline::new(source);
    outline
        .returned_leaves("createOperations")
        .into_iter()
        .map(|leaf| outline.function(&leaf.path, &leaf, false))
        .collect()
}

/// One function of a returned object: its path, what the object holds
/// there, and the body of the function that returns it, where a function it
/// names is looked for first.
struct Leaf {
    path: String,
    value: LeafValue,
    scope: Option<(usize, usize)>,
}

enum LeafValue {
    /// A function declared elsewhere: `{ deploy }`, `{ all: claimAll }`.
    Named(String),
    /// A function expression written in place, after the key starting at
    /// `key`: `{ deposit: instructionOperation(async (input) => ...) }`.
    Inline { key: usize, value: usize },
    /// A shorthand method whose key starts at `key` and whose parameter
    /// list opens at `open`: `{ limits(): T { ... } }`.
    Method { key: usize, open: usize },
    /// Anything else.
    Unknown,
}

/// A shorthand method in an object literal.
struct ShorthandMethod {
    name: String,
    /// Where its parameter list opens.
    open: usize,
}

/// A source and its code: the same bytes with every string, template
/// literal and comment blanked to spaces, so offsets match.
struct Outline<'a> {
    source: &'a str,
    code: String,
}

impl<'a> Outline<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            code: code_only(source),
        }
    }

    /// The leaves of the object the extension's `method` returns: the
    /// last object its body returns, or its arrow function's object
    /// expression (`createOperations: () => ({ ... })`).
    fn returned_leaves(&self, method: &str) -> Vec<Leaf> {
        let pattern = Regex::new(&format!(
            r"\b{}\s*(?::\s*(?:async\s+)?(?:function\s*)?)?\(",
            regex::escape(method)
        ))
        .expect("method regex should compile");
        let Some(found) = pattern.find(&self.code) else {
            return Vec::new();
        };
        let open = found.end() - 1;
        let (object, scope) = match self.function_body(open) {
            Some(Body::Block(body)) => (
                self.body_returns(body)
                    .into_iter()
                    .rev()
                    .find(|start| self.code.as_bytes()[*start] == b'{'),
                matching(&self.code, body).map(|close| (body, close)),
            ),
            Some(Body::Expression(start)) => (self.object_expression(start), None),
            None => (None, None),
        };
        let Some(object) = object else {
            return Vec::new();
        };
        let mut leaves = Vec::new();
        self.object_leaves(object, "", 0, &mut leaves);
        leaves.truncate(MAX_FUNCTIONS);
        for leaf in &mut leaves {
            leaf.scope = scope;
        }
        leaves
    }

    /// The helper namespaces (`addresses`, `math`, `constants`,
    /// `defaults`) the extension definition declares: each helper's path
    /// when the namespace is written in place, else the namespace alone.
    fn helpers(&self) -> Vec<String> {
        let pattern =
            Regex::new(r"\bdefine(?:Program|Stack)Extensions\s*(?:<[^()]*>)?\s*\(\s*\)\s*\(\s*\{")
                .expect("definition regex should compile");
        let Some(found) = pattern.find(&self.code) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        self.object_leaves(found.end() - 1, "", 0, &mut entries);
        let mut helpers = Vec::new();
        for entry in entries {
            let namespace = entry.path.split('.').next().unwrap_or_default();
            if !HELPER_NAMESPACES.contains(&namespace) {
                continue;
            }
            let helper = if entry.path.contains('.') {
                entry.path
            } else {
                namespace.to_string()
            };
            if !helpers.contains(&helper) {
                helpers.push(helper);
            }
        }
        helpers.truncate(MAX_FUNCTIONS);
        helpers
    }

    /// The shorthand method the object entry `code[start..end]` is: an
    /// optional `async`, `get`, `set` or `*`, a name (quoted or not),
    /// optional type parameters, then its parameter list and a body.
    fn shorthand_method(&self, start: usize, end: usize) -> Option<ShorthandMethod> {
        let method = Regex::new(
            r#"^(?:async\s+)?(?:(?:get|set)\s+)?\*?\s*([A-Za-z_$][\w$]*|'[^']*'|"[^"]*")\s*(?:<[^()]*>)?\s*\("#,
        )
        .expect("method regex should compile");
        let found = method.captures(&self.code[start..end])?;
        let open = start + found.get(0)?.end() - 1;
        // A method has a body; `key: value` never matches the regex.
        self.block_body(open)?;
        let name = self.source[start + found.get(1)?.start()..start + found.get(1)?.end()]
            .trim_matches(['\'', '"'])
            .to_string();
        Some(ShorthandMethod { name, open })
    }

    /// The object literal an expression starting at `start` is, through
    /// grouping parentheses: `({ ... })`.
    fn object_expression(&self, start: usize) -> Option<usize> {
        let mut at = start;
        while self.code.as_bytes().get(at) == Some(&b'(') {
            at = skip_whitespace(&self.code, at + 1);
        }
        (self.code.as_bytes().get(at) == Some(&b'{')).then_some(at)
    }

    /// The keys of a `readArgCounts: { ... }` declaration, flattened.
    fn read_arg_count_names(&self) -> Vec<Leaf> {
        let pattern =
            Regex::new(r"\breadArgCounts\s*:\s*\{").expect("read counts regex should compile");
        let Some(found) = pattern.find(&self.code) else {
            return Vec::new();
        };
        let mut leaves = Vec::new();
        self.object_leaves(found.end() - 1, "", 0, &mut leaves);
        leaves
            .into_iter()
            .map(|leaf| Leaf {
                value: LeafValue::Named(
                    leaf.path
                        .rsplit('.')
                        .next()
                        .unwrap_or(&leaf.path)
                        .to_string(),
                ),
                path: leaf.path,
                scope: None,
            })
            .collect()
    }

    /// The leaves of the object literal opening at `open`, under `prefix`.
    fn object_leaves(&self, open: usize, prefix: &str, depth: usize, leaves: &mut Vec<Leaf>) {
        let Some(close) = matching(&self.code, open) else {
            return;
        };
        for (start, end) in split_top_level(&self.code, open + 1, close) {
            let code = &self.code[start..end];
            let trimmed = code.trim();
            if trimmed.is_empty() || trimmed.starts_with("...") {
                continue;
            }
            let offset = start + (code.len() - code.trim_start().len());
            // A shorthand method, `limits(): T { ... }` or `async read() {}`:
            // its name, never its body.
            if let Some(method) = self.shorthand_method(offset, end) {
                let path = if prefix.is_empty() {
                    method.name.clone()
                } else {
                    format!("{prefix}.{}", method.name)
                };
                leaves.push(Leaf {
                    path,
                    value: LeafValue::Method {
                        key: offset,
                        open: method.open,
                    },
                    scope: None,
                });
                continue;
            }
            let colon = top_level_colon(&self.code, offset, end);
            let (key, value) = match colon {
                Some(colon) => (self.source[offset..colon].trim(), Some((colon + 1, end))),
                None => (self.source[offset..end].trim(), None),
            };
            let key = key.trim_matches(['\'', '"']);
            if key.is_empty() {
                continue;
            }
            let path = if prefix.is_empty() {
                key.to_string()
            } else {
                format!("{prefix}.{key}")
            };
            let value = match value {
                None if is_identifier(key) => LeafValue::Named(key.to_string()),
                None => LeafValue::Unknown,
                Some((value_start, value_end)) => {
                    let value = self.code[value_start..value_end].trim();
                    let value_offset = skip_whitespace(&self.code, value_start);
                    if value.starts_with('{') && depth < MAX_NESTING {
                        self.object_leaves(value_offset, &path, depth + 1, leaves);
                        continue;
                    }
                    if is_identifier(value) {
                        LeafValue::Named(value.to_string())
                    } else {
                        LeafValue::Inline {
                            key: offset,
                            value: value_offset,
                        }
                    }
                }
            };
            leaves.push(Leaf {
                path,
                value,
                scope: None,
            });
        }
    }

    /// The function a leaf holds, documented from its declaration.
    fn function(&self, path: &str, leaf: &Leaf, read: bool) -> FunctionReference {
        let mut function = FunctionReference {
            path: path.to_string(),
            params: Vec::new(),
            returns: None,
            title: None,
            description: None,
        };
        let declaration = match &leaf.value {
            LeafValue::Named(ident) => self.declaration(ident, leaf.scope),
            LeafValue::Method { key, open } => Some(Declaration {
                start: *key,
                params: self.params(*open).unwrap_or_default(),
                body: self.block_body(*open),
            }),
            LeafValue::Inline { key, value } => {
                self.function_expression(*value)
                    .map(|(params, body)| Declaration {
                        start: *key,
                        params,
                        body,
                    })
            }
            LeafValue::Unknown => None,
        };
        let Some(declaration) = declaration else {
            return function;
        };
        function.params = declaration.params;
        if let Some((title, description)) = self.doc_before(declaration.start) {
            function.title = title;
            function.description = description;
        }
        if read {
            function.returns = declaration.body.and_then(|body| self.returned_shape(body));
        }
        function
    }

    /// Where `ident` is declared as a function or as a constant holding
    /// one (`const deploy = instructionOperation(async (input: T) => {...})`):
    /// in `scope` when it declares it, at the shallowest nesting, since a
    /// function that returns `{ ident }` names its own declaration, not one
    /// local to another function.
    fn declaration(&self, ident: &str, scope: Option<(usize, usize)>) -> Option<Declaration> {
        let name = regex::escape(ident);
        let function = Regex::new(&format!(
            r"\b(?:async\s+)?function\s*\*?\s*{name}\s*(?:<[^>()]*>)?\s*\("
        ))
        .expect("function regex should compile");
        let constant = Regex::new(&format!(r"\b(?:const|let)\s+{name}\s*(?::[^=;]*)?=[^=>]"))
            .expect("constant regex should compile");
        let candidates = function
            .find_iter(&self.code)
            .map(|found| (found, true))
            .chain(constant.find_iter(&self.code).map(|found| (found, false)));
        let in_scope = |at: usize| scope.is_some_and(|(start, end)| start < at && at < end);
        let (found, is_function) = candidates.min_by_key(|(found, _)| {
            (
                !in_scope(found.start()),
                self.depth_at(found.start()),
                found.start(),
            )
        })?;
        if is_function {
            let open = found.end() - 1;
            return Some(Declaration {
                start: found.start(),
                params: self.params(open)?,
                body: self.block_body(open),
            });
        }
        let (params, body) = self.function_expression(found.end() - 1)?;
        Some(Declaration {
            start: found.start(),
            params,
            body,
        })
    }

    /// How many brackets are open at `at`.
    fn depth_at(&self, at: usize) -> usize {
        self.code.as_bytes()[..at]
            .iter()
            .fold(0usize, |depth, byte| match byte {
                b'{' | b'(' | b'[' => depth + 1,
                b'}' | b')' | b']' => depth.saturating_sub(1),
                _ => depth,
            })
    }

    /// The parameters and block body of the function expression at `at`,
    /// through at most a few wrapper calls:
    /// `instructionOperation(async (input: T) => { ... })`.
    fn function_expression(&self, mut at: usize) -> Option<(Vec<String>, Option<usize>)> {
        for _ in 0..4 {
            at = skip_whitespace(&self.code, at);
            let rest = &self.code[at..];
            if let Some(after) = rest.strip_prefix("async") {
                if after.starts_with(|c: char| c.is_whitespace() || c == '(') {
                    at += "async".len();
                    continue;
                }
            }
            if let Some(after) = rest.strip_prefix("function") {
                let open = skip_whitespace(&self.code, at + "function".len());
                if after.starts_with(|c: char| c.is_whitespace() || c == '(')
                    && self.code[open..].starts_with('(')
                {
                    return Some((self.params(open)?, self.block_body(open)));
                }
            }
            if rest.starts_with('(') {
                let close = matching(&self.code, at)?;
                let after = skip_whitespace(&self.code, close + 1);
                if self.code[after..].starts_with("=>") || self.code[after..].starts_with(':') {
                    return Some((self.params(at)?, self.block_body(at)));
                }
                return None;
            }
            let callee = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.'))
                .unwrap_or(rest.len());
            if callee == 0 {
                return None;
            }
            let after = skip_whitespace(&self.code, at + callee);
            if self.code[after..].starts_with("=>") {
                // `x => ...`: one untyped parameter.
                return Some((vec![self.code[at..at + callee].to_string()], None));
            }
            let after = skip_type_arguments(&self.code, after);
            if !self.code[after..].starts_with('(') {
                return None;
            }
            at = after + 1;
        }
        None
    }

    /// The parameters of the list opening at `open`, as declared, with
    /// defaults dropped (and the parameter marked optional).
    fn params(&self, open: usize) -> Option<Vec<String>> {
        let close = matching(&self.code, open)?;
        Some(
            split_top_level(&self.code, open + 1, close)
                .into_iter()
                .map(|(start, end)| {
                    let param = collapse_whitespace(&self.source[start..end]);
                    match top_level_default(&param) {
                        Some(at) => {
                            let declared = param[..at].trim();
                            match declared.split_once(':') {
                                Some((name, ty)) if !name.ends_with('?') => {
                                    format!("{}?:{}", name.trim(), ty.trim_end())
                                }
                                Some(_) => declared.to_string(),
                                None => format!("{declared}?"),
                            }
                        }
                        None => param,
                    }
                })
                .filter(|param| !param.is_empty())
                .collect(),
        )
    }

    /// The block body (`{`'s offset) of the function whose parameter list
    /// opens at `open`. `None` for an arrow function with an expression
    /// body.
    fn block_body(&self, open: usize) -> Option<usize> {
        match self.function_body(open)? {
            Body::Block(body) => Some(body),
            Body::Expression(_) => None,
        }
    }

    /// The body of the function whose parameter list opens at `open`: past
    /// an optional return type, and `=>` for an arrow function.
    fn function_body(&self, open: usize) -> Option<Body> {
        let close = matching(&self.code, open)?;
        let bytes = self.code.as_bytes();
        let mut at = skip_whitespace(&self.code, close + 1);
        let mut depth = 0usize;
        while at < bytes.len() {
            match bytes[at] {
                b'=' if depth == 0 && self.code[at..].starts_with("=>") => {
                    let start = skip_whitespace(&self.code, at + 2);
                    return Some(if bytes.get(start) == Some(&b'{') {
                        Body::Block(start)
                    } else {
                        Body::Expression(start)
                    });
                }
                b'{' if depth == 0 && !self.code[..at].trim_end().ends_with(':') => {
                    return Some(Body::Block(at));
                }
                b'(' | b'[' | b'{' | b'<' => depth += 1,
                b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
                b'>' => depth = depth.saturating_sub(1),
                b';' if depth == 0 => return None,
                _ => {}
            }
            at += 1;
        }
        None
    }

    /// Where the expression of each `return` of the function whose body
    /// opens at `open` starts: in its statement blocks too (`if`, `for`,
    /// `try`...), but not in the functions it declares.
    fn body_returns(&self, open: usize) -> Vec<usize> {
        let Some(close) = matching(&self.code, open) else {
            return Vec::new();
        };
        let bytes = self.code.as_bytes();
        let mut starts = Vec::new();
        // One entry per open bracket: whether a `return` directly inside it
        // returns from this function (a statement block does).
        let mut frames: Vec<bool> = Vec::new();
        let mut at = open + 1;
        while at < close {
            match bytes[at] {
                b'{' => frames.push(self.opens_block(at)),
                b'(' | b'[' => frames.push(false),
                b'}' | b')' | b']' => {
                    frames.pop();
                }
                b'r' if frames.iter().all(|block| *block)
                    && self.code[at..].starts_with("return")
                    && !is_identifier_byte(bytes[at - 1])
                    && bytes
                        .get(at + "return".len())
                        .is_some_and(|next| !is_identifier_byte(*next)) =>
                {
                    let start = skip_whitespace(&self.code, at + "return".len());
                    if start < close {
                        starts.push(start);
                    }
                    at += "return".len();
                    continue;
                }
                _ => {}
            }
            at += 1;
        }
        starts
    }

    /// Whether the `{` at `at` opens a statement block, rather than a
    /// function body or an object literal.
    fn opens_block(&self, at: usize) -> bool {
        let before = self.code[..at].trim_end();
        if before.ends_with("=>") {
            return false;
        }
        if before.ends_with(')') {
            let Some(open) = matching_back(&self.code, before.len() - 1) else {
                return false;
            };
            let keyword = self.code[..open]
                .trim_end()
                .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                .next()
                .unwrap_or_default();
            return matches!(
                keyword,
                "if" | "for" | "while" | "switch" | "catch" | "with"
            );
        }
        let word = before
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
            .next()
            .unwrap_or_default();
        matches!(word, "else" | "try" | "finally" | "do")
            || before.is_empty()
            || before.ends_with([';', '{', '}'])
    }

    /// What a read resolves to, from the `return`s of its body: object
    /// keys, `null`, a state view's row or a program account. `None` when a
    /// `return` is anything else.
    fn returned_shape(&self, body: usize) -> Option<String> {
        let row =
            Regex::new(r"^[\w.]*\bviews\.(\w+)\.state\.get\(").expect("view regex should compile");
        let account =
            Regex::new(r"^[\w.]*\baccounts\.(\w+)\.fetch\(").expect("account regex should compile");
        let mut shapes: Vec<String> = Vec::new();
        let returns = self.body_returns(body);
        if returns.is_empty() {
            return None;
        }
        for start in returns {
            let rest = &self.code[start..];
            let shape = if rest.starts_with('{') {
                let close = matching(&self.code, start)?;
                let keys = split_top_level(&self.code, start + 1, close)
                    .into_iter()
                    .filter_map(|(key_start, key_end)| {
                        let entry = self.source[key_start..key_end].trim();
                        let key = match top_level_colon(&self.code, key_start, key_end) {
                            Some(colon) => self.source[key_start..colon].trim(),
                            None => entry,
                        };
                        (!key.is_empty()).then(|| key.to_string())
                    })
                    .collect::<Vec<_>>();
                format!("{{ {} }}", keys.join(", "))
            } else if rest.starts_with("null") {
                "null".to_string()
            } else {
                // A state view and an account fetch both resolve to null
                // when there is nothing at the key.
                let captures = row.captures(rest).or_else(|| account.captures(rest))?;
                format!("{} | null", &captures[1])
            };
            for part in shape_parts(&shape) {
                if !shapes.contains(&part) {
                    shapes.push(part);
                }
            }
        }
        // `null` last, as TypeScript writes it.
        shapes.sort_by_key(|shape| shape == "null");
        Some(shapes.join(" | "))
    }

    /// The `@title` and description of the JSDoc block right before
    /// `start`, past an `export`.
    fn doc_before(&self, start: usize) -> Option<(Option<String>, Option<String>)> {
        let before = self.source[..start].trim_end();
        let before = before.strip_suffix("export").unwrap_or(before).trim_end();
        let before = before.strip_suffix("*/")?;
        let open = before.rfind("/**")?;
        Some(parse_doc(&before[open + 3..]))
    }
}

/// Where a function's body starts.
enum Body {
    /// `{ ... }`.
    Block(usize),
    /// An arrow function's expression.
    Expression(usize),
}

/// A function declaration: where it starts, its parameters and its body.
struct Declaration {
    start: usize,
    params: Vec<String>,
    body: Option<usize>,
}

/// `A | null` → `["A", "null"]`, keeping objects whole.
fn shape_parts(shape: &str) -> Vec<String> {
    if shape.starts_with('{') {
        return vec![shape.to_string()];
    }
    shape.split(" | ").map(str::to_string).collect()
}

/// A JSDoc body's `@title`, and its description: the text before the
/// first tag, as one paragraph.
fn parse_doc(comment: &str) -> (Option<String>, Option<String>) {
    let lines = comment
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .collect::<Vec<_>>();
    let title = lines
        .iter()
        .find_map(|line| line.strip_prefix("@title "))
        .map(|title| title.trim().to_string());
    let description = lines
        .iter()
        .take_while(|line| !line.starts_with('@'))
        .filter(|line| !line.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    (title, (!description.is_empty()).then_some(description))
}

/// `source` with every string, template literal and comment replaced by
/// spaces, byte for byte (line breaks kept).
fn code_only(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut code = bytes.to_vec();
    let mut blank = |range: std::ops::Range<usize>| {
        for byte in &mut code[range] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                let end = bytes[at..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(bytes.len(), |offset| at + offset);
                blank(at..end);
                at = end;
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                let end = source[at + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |offset| at + 2 + offset + 2);
                blank(at..end);
                at = end;
            }
            quote @ (b'\'' | b'"' | b'`') => {
                let mut end = at + 1;
                while end < bytes.len() {
                    match bytes[end] {
                        b'\\' => end += 2,
                        byte if byte == quote => {
                            end += 1;
                            break;
                        }
                        b'\n' if quote != b'`' => break,
                        _ => end += 1,
                    }
                }
                let end = end.min(bytes.len());
                // Keep the quotes, so a quoted key is still a key.
                if end > at + 1 {
                    blank(at + 1..end.saturating_sub(1).max(at + 1));
                }
                at = end;
            }
            _ => at += 1,
        }
    }
    // Only ASCII bytes were blanked, and only whole characters.
    String::from_utf8_lossy(&code).into_owned()
}

/// The bracket closing the one opening at `open`.
fn matching(code: &str, open: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let (opening, closing) = match bytes.get(open)? {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0usize;
    for (at, byte) in bytes.iter().enumerate().skip(open) {
        if *byte == opening {
            depth += 1;
        } else if *byte == closing {
            depth -= 1;
            if depth == 0 {
                return Some(at);
            }
        }
    }
    None
}

/// The ranges of `code[start..end]` between its top-level commas.
fn split_top_level(code: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let bytes = code.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut from = start;
    for (at, byte) in bytes.iter().enumerate().take(end).skip(start) {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push((from, at));
                from = at + 1;
            }
            _ => {}
        }
    }
    parts.push((from, end));
    parts
        .into_iter()
        .filter(|(start, end)| !code[*start..*end].trim().is_empty())
        .collect()
}

/// The first `:` of `code[start..end]` outside brackets.
fn top_level_colon(code: &str, start: usize, end: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    for (at, byte) in bytes.iter().enumerate().take(end).skip(start) {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 => return Some(at),
            _ => {}
        }
    }
    None
}

/// Where a parameter's default value (`= ...`) starts, outside brackets
/// and not as part of `=>`, `==` or a comparison.
fn top_level_default(param: &str) -> Option<usize> {
    let bytes = param.as_bytes();
    let mut depth = 0usize;
    for (at, byte) in bytes.iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'>' if at == 0 || bytes[at - 1] != b'=' => depth = depth.saturating_sub(1),
            b'=' if depth == 0
                && !matches!(bytes.get(at + 1), Some(b'>' | b'='))
                && !matches!(
                    at.checked_sub(1).map(|before| bytes[before]),
                    Some(b'!' | b'<' | b'>' | b'=')
                ) =>
            {
                return Some(at);
            }
            _ => {}
        }
    }
    None
}

/// `<...>` type arguments at `at`, skipped.
fn skip_type_arguments(code: &str, at: usize) -> usize {
    let bytes = code.as_bytes();
    if bytes.get(at) != Some(&b'<') {
        return at;
    }
    let mut depth = 0usize;
    for (offset, byte) in bytes[at..].iter().enumerate() {
        match byte {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return skip_whitespace(code, at + offset + 1);
                }
            }
            _ => {}
        }
    }
    at
}

/// The bracket opening the one closing at `close`.
fn matching_back(code: &str, close: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let (opening, closing) = match bytes.get(close)? {
        b'}' => (b'{', b'}'),
        b')' => (b'(', b')'),
        b']' => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0usize;
    for at in (0..=close).rev() {
        if bytes[at] == closing {
            depth += 1;
        } else if bytes[at] == opening {
            depth -= 1;
            if depth == 0 {
                return Some(at);
            }
        }
    }
    None
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn skip_whitespace(code: &str, at: usize) -> usize {
    at + code[at..].len() - code[at..].trim_start().len()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of ORE's published stack extension.
    const STACK_EXTENSION: &str = r#"
import { defineStackExtensions } from '@usearete/sdk';

export default defineStackExtensions<typeof CORE>()({
  readArgCounts: {
    boardState: 0,
    roundState: 1,
    currentRound: 0,
  },
  createRead(client) {
    const program = client.programs.ore as OreProgramRuntime;

    /**
     * Reads the `OreBoard` view entity for the canonical Board PDA.
     *
     * @title Board state
     * @concept mining
     */
    async function boardState() {
      return client.views.OreBoard.state.get({
        address: program.addresses.board(),
      });
    }

    /**
     * Reads the `OreRound` view entity for a specific round id.
     *
     * @title Round state
     */
    async function roundState(roundId: bigint) {
      return client.views.OreRound.state.get({ roundId });
    }

    /**
     * Reads the Board entity and the chain clock together. Returns null
     * when the Board entity is missing ("{" in a string is not code).
     *
     * @title Current round
     */
    async function currentRound() {
      const [boardEntity, clock] = await Promise.all([boardState(), client.chain.clock()]);
      if (!boardEntity) {
        return null;
      }
      const roundEntity = await roundState(boardEntity.state.roundId);
      return {
        board: boardEntity,
        round: roundEntity,
        roundAddress: program.addresses.round(1n),
        clock,
        phase: program.math.round.phase({ startSlot: 1n, endSlot: 2n }, 3n),
      };
    }

    return {
      boardState,
      roundState,
      currentRound,
    };
  },
});
"#;

    /// The shape of ORE's published program extension.
    const PROGRAM_EXTENSION: &str = r#"
export const oreProgramExtensions = defineProgramExtensions<typeof ORE>()({
  addresses,
  createRead(context) {
    const program = context.program;

    /**
     * Fetches an authority's Miner account, deriving the
     * `["miner", authority]` PDA.
     *
     * @title Read miner
     */
    async function miner(authority: Address) {
      return program.accounts.Miner.fetch(addresses.miner(authority));
    }

    return { miner };
  },
  createOperations(context) {
    /**
     * Deploys SOL to the selected squares of the current round.
     *
     * @title Deploy to squares
     */
    const deploy = instructionOperation(async (input: DeploySemanticInput) => {
      return prepared(input);
    });

    /**
     * @title Claim all rewards
     */
    const claimAll = transactionOperation(
      async (input: { authority: Address; tip?: bigint }, extra = 1) => {
        return createPreparedTransaction({ name: 'claimAll' });
      },
    );

    return {
      instructions: {
        mining: { deploy },
        rewards: { all: claimAll, 'odd-key': claimAll },
      },
      transactions: { rewards: { claimAll } },
    };
  },
});
"#;

    #[test]
    fn stack_reads_have_their_signature_title_description_and_return_shape() {
        let reads = reads(STACK_EXTENSION);
        assert_eq!(
            reads
                .iter()
                .map(FunctionReference::signature)
                .collect::<Vec<_>>(),
            [
                "read.boardState()",
                "read.roundState(roundId: bigint)",
                "read.currentRound()"
            ]
        );
        assert_eq!(reads[0].title.as_deref(), Some("Board state"));
        assert_eq!(
            reads[0].description.as_deref(),
            Some("Reads the `OreBoard` view entity for the canonical Board PDA.")
        );
        assert_eq!(reads[0].returns.as_deref(), Some("OreBoard | null"));
        assert_eq!(reads[1].returns.as_deref(), Some("OreRound | null"));
        assert_eq!(
            reads[2].returns.as_deref(),
            Some("{ board, round, roundAddress, clock, phase } | null")
        );
        assert_eq!(
            reads[2].description.as_deref(),
            Some(
                "Reads the Board entity and the chain clock together. Returns null when the Board entity is missing (\"{\" in a string is not code)."
            )
        );
    }

    #[test]
    fn program_reads_and_operations_keep_their_namespaces() {
        let reads = reads(PROGRAM_EXTENSION);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].signature(), "read.miner(authority: Address)");
        assert_eq!(reads[0].returns.as_deref(), Some("Miner | null"));

        let operations = operations(PROGRAM_EXTENSION);
        assert_eq!(
            operations
                .iter()
                .map(FunctionReference::signature)
                .collect::<Vec<_>>(),
            [
                "instructions.mining.deploy(input: DeploySemanticInput)",
                "instructions.rewards.all(input: { authority: Address; tip?: bigint }, extra?)",
                "instructions.rewards.odd-key(input: { authority: Address; tip?: bigint }, extra?)",
                "transactions.rewards.claimAll(input: { authority: Address; tip?: bigint }, extra?)",
            ]
        );
        assert_eq!(operations[0].title.as_deref(), Some("Deploy to squares"));
        assert_eq!(
            operations[0].description.as_deref(),
            Some("Deploys SOL to the selected squares of the current round.")
        );
        // Operations are prepared, not read: no return shape.
        assert_eq!(operations[0].returns, None);
        assert_eq!(operations[1].title.as_deref(), Some("Claim all rewards"));
        assert_eq!(operations[1].description, None);
    }

    #[test]
    fn without_a_returned_object_the_declared_read_counts_name_the_reads() {
        let source = r#"
export default defineStackExtensions()({
  readArgCounts: { latest: 0, byId: [1, 2] },
  createRead: (client) => buildReads(client),
});
/** @title Latest */
function latest() { return client.views.Thing.state.get({ id: 1 }); }
"#;
        let reads = reads(source);
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].path, "read.latest");
        assert_eq!(reads[0].title.as_deref(), Some("Latest"));
        assert_eq!(reads[1].path, "read.byId");
        assert!(reads[1].params.is_empty());
    }

    #[test]
    fn operations_written_in_place_in_an_arrow_function_object_are_listed() {
        let source = r#"
export default defineProgramExtensions<typeof VAULT>()({
  defaults: { fee: () => 1n },
  createOperations: () => ({
    instructions: {
      treasury: {
        /**
         * Deposits into the treasury vault.
         *
         * @title Deposit to treasury
         */
        deposit: instructionOperation(async (input: DepositToTreasuryInput) => {
          return createPreparedInstruction({ name: 'treasury.deposit' });
        }),
        sweep: transactionOperation(function (input: SweepInput) {
          return build(input);
        }),
      },
    },
  }),
});
"#;
        let operations = operations(source);
        assert_eq!(
            operations
                .iter()
                .map(FunctionReference::signature)
                .collect::<Vec<_>>(),
            [
                "instructions.treasury.deposit(input: DepositToTreasuryInput)",
                "instructions.treasury.sweep(input: SweepInput)",
            ]
        );
        assert_eq!(operations[0].title.as_deref(), Some("Deposit to treasury"));
        assert_eq!(
            operations[0].description.as_deref(),
            Some("Deposits into the treasury vault.")
        );
        assert_eq!(operations[1].title, None);
    }

    #[test]
    fn a_returned_name_is_its_own_declaration_not_a_local_of_another_function() {
        let source = r#"
export default defineProgramExtensions<typeof ORE>()({
  addresses,
  math: { phase, roundFee: (round: bigint) => round },
  constants: { program: ORE_ID },
  createRead(context) {
    async function checkpointPreview(authority: Address) {
      const checkpoint = math.preview(authority);
      return { checkpoint };
    }
    return { checkpointPreview };
  },
  createOperations(context) {
    /** @title Checkpoint miner round */
    const checkpoint = instructionOperation(
      async (input: CheckpointSemanticInput) => prepare(input),
    );
    return { instructions: { miner: { checkpoint } } };
  },
});
"#;
        let operations = operations(source);
        assert_eq!(
            operations[0].signature(),
            "instructions.miner.checkpoint(input: CheckpointSemanticInput)"
        );
        assert_eq!(
            operations[0].title.as_deref(),
            Some("Checkpoint miner round")
        );
        assert_eq!(
            helpers(source),
            [
                "addresses",
                "math.phase",
                "math.roundFee",
                "constants.program"
            ]
        );
    }

    #[test]
    fn shorthand_methods_are_listed_by_name_with_their_signature() {
        let source = r#"
export default defineStackExtensions<typeof CORE>()({
  defaults: {
    limits(): VaultLimits {
      return { maxDeposit: 1_000_000n };
    },
    fee() { return { bps: 30 }; },
    async latest<T>(id: T) { return null; },
    'odd-key'(x: number) { return x; },
    plain: 1,
  },
  createRead(client) {
    return {
      /** @title Vault */
      vault(address: string) {
        return client.views.Vault.state.get({ address });
      },
    };
  },
});
"#;
        assert_eq!(
            helpers(source),
            [
                "defaults.limits",
                "defaults.fee",
                "defaults.latest",
                "defaults.odd-key",
                "defaults.plain"
            ]
        );
        let reads = reads(source);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].signature(), "read.vault(address: string)");
        assert_eq!(reads[0].title.as_deref(), Some("Vault"));
        assert_eq!(reads[0].returns.as_deref(), Some("Vault | null"));
    }

    #[test]
    fn an_extension_without_reads_or_operations_lists_none() {
        assert!(reads("export const x = 1;").is_empty());
        assert!(operations("export default defineStackExtensions()({});").is_empty());
    }
}
