//
// Pure parsing of the TapHunter "bigscreen" menu. No network or `worker`
// dependency, so it builds and tests on the host target.
//

use crate::error::{AppError, AppResult};
use crate::model::BeerEntry;
use chrono::NaiveDateTime;
use regex::Regex;
use serde_json::Value;

pub const BASE_TAPHUNTER_URL: &str = "http://www.taphunter.com/bigscreen";
/// Beer Thirty's TapHunter venue id.
pub const BEER_THIRTY_VENUE_ID: &str = "5469327503392768";

/// URL of the bigscreen HTML page that embeds the JSON endpoint.
pub fn bigscreen_url() -> String {
    format!("{BASE_TAPHUNTER_URL}/{BEER_THIRTY_VENUE_ID}")
}

/// Extract the absolute JSON menu URL from the bigscreen page HTML. The page
/// contains a `getJSON('json/<token>')` call whose token we resolve to a full
/// URL.
pub fn parse_json_url(html: &str) -> AppResult<String> {
    let re = Regex::new(r#"getJSON\(['"](\./)?json/([^'"]+)['"]"#)
        .map_err(|e| AppError::Parse(format!("Regex creation failed: {e}")))?;

    let captures = re
        .captures(html)
        .ok_or_else(|| AppError::Parse("Could not find getJSON URL".into()))?;
    let relative_path = captures
        .get(2)
        .ok_or_else(|| AppError::Parse("Failed to capture JSON path".into()))?
        .as_str();

    Ok(format!("{BASE_TAPHUNTER_URL}/json/{relative_path}"))
}

/// Parse the TapHunter menu JSON into beer entries.
pub fn parse_menu(json: &str) -> AppResult<Vec<BeerEntry>> {
    let items: Vec<Value> = serde_json::from_str(json)
        .map_err(|e| AppError::Parse(format!("Failed to parse menu JSON: {e}")))?;

    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        let date_str = item["date_added"].as_str().unwrap_or("");
        let days_old = calculate_days_old(date_str).unwrap_or(0);

        let abv = clean_text(item["beer"]["abv"].as_str().unwrap_or(""));
        let mut entry = BeerEntry {
            tap_number: item["serving_info"]["tap_number"].as_i64().unwrap_or(0) as i32,
            brewery: clean_text(item["brewery"]["common_name"].as_str().unwrap_or("")),
            name: clean_text(item["beer"]["beer_name"].as_str().unwrap_or("")),
            abv: if abv.is_empty() {
                "0.0".to_string()
            } else {
                abv
            },
            category: clean_text(item["beer"]["style_category"].as_str().unwrap_or("")),
            origin: clean_text(item["brewery"]["origin"].as_str().unwrap_or("")),
            style: clean_text(item["beer"]["style"].as_str().unwrap_or("")),
            days_old: days_old as i32,
        };

        // Strip "**Nitro**" markers used to flag nitro taps.
        entry.brewery = entry.brewery.replace("**Nitro**", "").trim().to_string();
        entry.name = entry
            .name
            .replace("**NITRO**", "")
            .replace("**Nitro**", "")
            .replace("NITRO", "")
            .replace("Nitro", "")
            .trim()
            .to_string();

        // Remove the brewery name from the beer name when duplicated.
        if !entry.brewery.is_empty() {
            entry.name = entry.name.replace(&entry.brewery, "").trim().to_string();
        }

        entries.push(entry);
    }

    Ok(entries)
}

/// Collapse runs of whitespace and tidy up stray spaces before commas.
pub(crate) fn clean_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .replace(" ,", ",")
        .trim()
        .to_string()
}

/// Days between `date_str` (MM/DD/YYYY) and now.
fn calculate_days_old(date_str: &str) -> AppResult<i64> {
    NaiveDateTime::parse_from_str(&format!("{date_str} 00:00:00"), "%m/%d/%Y %H:%M:%S")
        .map_err(|e| AppError::Parse(format!("Failed to parse date: {e}")))
        .map(|date| {
            let now = chrono::Local::now().naive_local();
            (now - date).num_days()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_url_from_bigscreen_fixture() {
        let html = include_str!("fixtures/taphunter_bigscreen.html");
        let url = parse_json_url(html).expect("should find getJSON url");
        assert!(url.starts_with("http://www.taphunter.com/bigscreen/json/"));
        assert!(url.len() > "http://www.taphunter.com/bigscreen/json/".len());
    }

    #[test]
    fn parse_json_url_errors_when_absent() {
        assert!(parse_json_url("<html>no script here</html>").is_err());
    }

    #[test]
    fn parses_menu_fixture() {
        let json = include_str!("fixtures/taphunter_menu.json");
        let entries = parse_menu(json).expect("should parse menu");
        assert!(!entries.is_empty(), "fixture has taps");

        let first = &entries[0];
        assert!(!first.brewery.is_empty());
        assert!(!first.name.is_empty());
        // ABV is always populated (defaults to "0.0").
        assert!(!first.abv.is_empty());
        // Every entry parsed a tap number.
        assert!(entries.iter().all(|e| e.tap_number >= 0));
    }

    #[test]
    fn clean_text_collapses_whitespace() {
        assert_eq!(
            clean_text("  Sierra   Nevada \n Pale  Ale "),
            "Sierra Nevada Pale Ale"
        );
    }
}
