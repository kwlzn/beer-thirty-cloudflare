//
// Untappd ratings provider.
//
// Untappd's search page no longer renders results server-side; it hydrates an
// `#algolia-hits` container from Algolia's hosted search API. So instead of
// scraping HTML we query that JSON API directly, which is far more stable. See
// TRIAGE.md for how the breakage was diagnosed and the credentials sourced.
//
// This module is pure (no network, no `worker` crate): it builds the request
// description and parses the response. The wasm worker and the native dev
// runner perform the actual HTTP and share these functions.
//

use crate::error::{AppError, AppResult};
use crate::model::RatingResult;
use regex::Regex;

pub const BASE_UNTAPPD_URL: &str = "https://untappd.com";

/// Public, client-side Algolia credentials embedded in untappd.com's search
/// page. If these rotate, `extract_algolia_credentials` can recover the current
/// values from a freshly fetched search page (see the wasm fallback).
pub const ALGOLIA_APP_ID: &str = "9WBO4RQ3HO";
pub const ALGOLIA_API_KEY: &str = "1d347324d67ec472bb7132c66aead485";
/// Default beer relevance index.
pub const ALGOLIA_INDEX: &str = "beer";

/// A ready-to-send Algolia query: POST `url` with the auth headers and `body`.
#[derive(Debug, Clone)]
pub struct AlgoliaQuery {
    pub url: String,
    pub app_id: String,
    pub api_key: String,
    pub body: String,
}

/// Build an Algolia query for a beer search string, using the given credentials.
pub fn build_query_with(app_id: &str, api_key: &str, search: &str) -> AlgoliaQuery {
    let url = format!("https://{app_id}-dsn.algolia.net/1/indexes/{ALGOLIA_INDEX}/query");
    // Only need the top hit; serde_json keeps the body simple and injection-safe.
    //
    // `ignorePlurals` handles imprecise menu names like "Strawberries" vs
    // "Strawberry" with no downside. We deliberately do NOT use
    // `removeWordsIfNoResults` here: trimming the query until *something* matches
    // force-matches beers that aren't really on Untappd (e.g. a kombucha sharing
    // a stray word with a real beer), and a wrong rating is worse than "N/A".
    let body = serde_json::json!({
        "query": search,
        "hitsPerPage": 1,
        "ignorePlurals": true,
    })
    .to_string();
    AlgoliaQuery {
        url,
        app_id: app_id.to_string(),
        api_key: api_key.to_string(),
        body,
    }
}

/// Build an Algolia query using the hardcoded credentials.
pub fn build_query(search: &str) -> AlgoliaQuery {
    build_query_with(ALGOLIA_APP_ID, ALGOLIA_API_KEY, search)
}

/// Parse an Algolia beer-query response into a rating + review link.
///
/// - Algolia auth/credential errors (`{"status": 4xx, ...}`) → `Blocked`, so the
///   caller skips caching and can trigger the credential-refresh fallback.
/// - No hits → `NotFound`. A hit with no rating yet still resolves (link to the
///   page, with "N/A" as the rating text).
pub fn parse_rating(body: &str) -> AppResult<RatingResult> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| AppError::Parse(format!("Failed to parse Algolia JSON: {e}")))?;

    // Algolia error envelopes carry a non-2xx `status` and a `message`.
    if let Some(status) = value.get("status").and_then(|s| s.as_u64()) {
        if !(200..300).contains(&status) {
            let msg = value
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(AppError::Blocked(format!("Algolia status {status}: {msg}")));
        }
    }

    let hits = value
        .get("hits")
        .and_then(|h| h.as_array())
        .ok_or_else(|| AppError::Parse("Algolia response missing 'hits'".into()))?;

    let hit = hits.first().ok_or(AppError::NotFound)?;

    // The beer exists on Untappd, so always build a link to its page.
    let slug = hit.get("beer_slug").and_then(|s| s.as_str()).unwrap_or("");
    let bid = hit
        .get("bid")
        .and_then(|b| b.as_i64())
        .ok_or_else(|| AppError::Parse("hit missing 'bid'".into()))?;
    let url = format!("{BASE_UNTAPPD_URL}/b/{slug}/{bid}");

    // Beers with too few check-ins come back with a 0 score: show "N/A" for the
    // rating but still link to the page.
    let rating = match hit.get("rating_score").and_then(|r| r.as_f64()) {
        Some(score) if score > 0.0 => format!("{score:.2}"),
        _ => "N/A".to_string(),
    };

    Ok(RatingResult { rating, url })
}

/// URL of the Untappd search page (used only by the credential-refresh fallback).
pub fn search_page_url(query: &str) -> String {
    format!(
        "{}/search?q={}",
        BASE_UNTAPPD_URL,
        url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
    )
}

/// Recover the current Algolia (app_id, api_key) from a fetched search page, in
/// case the hardcoded credentials rotate.
pub fn extract_algolia_credentials(html: &str) -> Option<(String, String)> {
    let app_re = Regex::new(r#"[aA]ppId["']?\s*[:=]\s*["']([A-Za-z0-9]{6,})"#).ok()?;
    let key_re = Regex::new(r#"apiKey["']?\s*[:=]\s*["']([a-f0-9]{16,})"#).ok()?;
    let app = app_re.captures(html)?.get(1)?.as_str().to_string();
    let key = key_re.captures(html)?.get(1)?.as_str().to_string();
    Some((app, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_query_targets_dsn_endpoint() {
        let q = build_query("sierra nevada pale ale");
        assert_eq!(
            q.url,
            "https://9WBO4RQ3HO-dsn.algolia.net/1/indexes/beer/query"
        );
        assert!(q.body.contains("sierra nevada pale ale"));
        assert!(q.body.contains("hitsPerPage"));
        // Fuzzy-matching control for imprecise menu names (plurals).
        assert!(q.body.contains("ignorePlurals"));
        // We do NOT trim words to force matches (avoids false positives).
        assert!(!q.body.contains("removeWordsIfNoResults"));
    }

    #[test]
    fn parses_rating_from_fixture() {
        let body = include_str!("fixtures/algolia_beer_query.json");
        let result = parse_rating(body).expect("should extract a rating");
        assert_eq!(result.rating, "3.62");
        assert_eq!(
            result.url,
            "https://untappd.com/b/sierra-nevada-brewing-co-pale-ale/6284"
        );
        // The rendered cell is a clickable link.
        assert!(result
            .to_cell()
            .starts_with("<a href=\"https://untappd.com/b/"));
        assert!(result.to_cell().ends_with("</a>"));
    }

    #[test]
    fn unrated_but_existing_beer_still_links() {
        let body = include_str!("fixtures/algolia_unrated.json");
        let result = parse_rating(body).expect("an existing beer should resolve");
        assert_eq!(result.rating, "N/A");
        assert_eq!(
            result.url,
            "https://untappd.com/b/ghost-town-brewing-sun-fade/6755584"
        );
        // The rendered cell is a clickable "N/A".
        assert_eq!(
            result.to_cell(),
            "<a href=\"https://untappd.com/b/ghost-town-brewing-sun-fade/6755584\">N/A</a>"
        );
    }

    #[test]
    fn no_hits_is_not_found() {
        let body = include_str!("fixtures/algolia_no_results.json");
        assert_eq!(parse_rating(body), Err(AppError::NotFound));
    }

    #[test]
    fn auth_error_is_blocked() {
        let body = include_str!("fixtures/algolia_auth_error.json");
        match parse_rating(body) {
            Err(AppError::Blocked(_)) => {}
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn recovers_credentials_from_search_page() {
        let html = include_str!("fixtures/untappd_search.html");
        let (app, key) = extract_algolia_credentials(html).expect("creds in page");
        assert_eq!(app, ALGOLIA_APP_ID);
        assert_eq!(key, ALGOLIA_API_KEY);
    }
}
