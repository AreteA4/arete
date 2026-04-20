//! Utility functions

/// Convert PascalCase or camelCase to snake_case
pub fn to_snake_case(s: &str) -> String {
    let mut result = String::new();

    for c in s.chars() {
        if c.is_uppercase() {
            if !result.is_empty() {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }

    result
}

/// Convert snake_case to PascalCase
pub fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_case_conversion() {
        assert_eq!(to_snake_case("PoolState"), "pool_state");
        assert_eq!(to_snake_case("LbPair"), "lb_pair");
        assert_eq!(to_snake_case("initialize"), "initialize");
        assert_eq!(to_snake_case("MyIDL"), "my_i_d_l");

        assert_eq!(to_pascal_case("pool_state"), "PoolState");
        assert_eq!(to_pascal_case("lb_pair"), "LbPair");
        assert_eq!(to_pascal_case("initialize"), "Initialize");
    }
}
