#[cfg(not(feature = "local"))]
const DEFAULT_API_URL: &str = "https://api.arete.run";

#[cfg(feature = "local")]
const DEFAULT_API_URL: &str = "http://localhost:3000";

/// Get the API URL from CLI override, environment variable, or use default.
pub fn get_api_url(override_url: Option<&str>) -> String {
    override_url
        .map(str::to_string)
        .or_else(|| std::env::var("ARETE_API_URL").ok())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string())
}

pub fn to_kebab_case(value: &str) -> String {
    let mut result = String::new();
    for (index, character) in value.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                result.push('-');
            }
            result.extend(character.to_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_kebab_case() {
        assert_eq!(to_kebab_case("SettlementGame"), "settlement-game");
        assert_eq!(to_kebab_case("OreRound"), "ore-round");
        assert_eq!(to_kebab_case("PumpfunToken"), "pumpfun-token");
        assert_eq!(to_kebab_case("simple"), "simple");
        assert_eq!(to_kebab_case("ABC"), "a-b-c");
    }
}
