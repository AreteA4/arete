//! Pointers for top-level command names that are not a4 commands but that
//! people and agents reach for, such as `a4 read` or `a4 query`: instead of a
//! bare "unrecognized subcommand", say which command (or `a4 mcp`, or an
//! SDK) does that job.

/// The tips for `name`, typed as a top-level command that a4 does not have;
/// `None` for a name with no known replacement (clap's error is shown).
pub fn unknown_command_tips(name: &str) -> Option<&'static [&'static str]> {
    const READ: &[&str] = &[
        "read a stack view once: `a4 get <Entity>/<view> --stack <stack>` (`--where`, `--key`, `--select`, `--limit`)",
        "follow it live: `a4 stream <Entity>/<view> --stack <stack>`",
        "from an agent: `a4 mcp` serves the `read_view` and `subscribe` tools",
        "from an app, with the `install` command group: install the stack's SDK with `a4 install stack <stack> --ts`",
    ];
    const STREAM: &[&str] = &[
        "follow a stack view live: `a4 stream <Entity>/<view> --stack <stack>`",
        "from an agent: `a4 mcp` serves the `subscribe` tool",
        "from an app, with the `install` command group: install the stack's SDK with `a4 install stack <stack> --ts`",
    ];
    const SEARCH: &[&str] = &[
        "search the catalog by intent: `a4 explore catalog --query \"<what you want to do>\"`",
        "list stacks: `a4 explore stacks`; list programs: `a4 explore programs`",
        "search the knowledge layer: `a4 know search --query \"<intent>\"`",
    ];
    const LIST: &[&str] = &[
        "list registry stacks: `a4 explore stacks`; list programs: `a4 explore programs`",
        "list your own stacks: `a4 stack list`",
    ];
    Some(match name.to_ascii_lowercase().as_str() {
        "read" | "query" | "view" | "views" | "fetch" | "entity" | "entities" => READ,
        "subscribe" | "watch" | "tail" | "follow" => STREAM,
        "search" | "find" | "discover" | "catalog" => SEARCH,
        "list" | "ls" | "stacks" => LIST,
        _ => return None,
    })
}

/// The error `a4 <name>` prints when `name` has [`unknown_command_tips`], in
/// clap's error layout.
pub fn unknown_command_message(name: &str) -> Option<String> {
    let tips = unknown_command_tips(name)?;
    let mut message = format!("error: '{name}' is not an a4 command\n\n");
    for tip in tips {
        message.push_str(&format!("  tip: {tip}\n"));
    }
    message.push_str("\nFor more information, try '--help'.\n");
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_like_names_point_to_get_stream_mcp_and_the_sdk() {
        for name in ["read", "query", "view", "Query"] {
            let message = unknown_command_message(name).expect(name);
            assert!(message.starts_with(&format!("error: '{name}' is not an a4 command")));
            for pointer in ["a4 get ", "a4 stream ", "a4 mcp", "a4 install stack"] {
                assert!(
                    message.contains(pointer),
                    "{name}: {pointer} missing\n{message}"
                );
            }
            assert!(message.ends_with("For more information, try '--help'.\n"));
        }
        assert!(unknown_command_message("subscribe")
            .unwrap()
            .contains("a4 stream "));
        assert!(unknown_command_message("search")
            .unwrap()
            .contains("a4 explore catalog --query"));
        assert!(unknown_command_message("list")
            .unwrap()
            .contains("a4 explore stacks"));
    }

    #[test]
    fn other_names_keep_clap_error() {
        assert!(unknown_command_message("instal").is_none());
        assert!(unknown_command_message("frobnicate").is_none());
    }
}
