//
// Beer Thirty tap-menu worker.
//
// The parsing/rendering logic lives in the pure modules below (no `worker`
// dependency), so it compiles and unit-tests on the host with `cargo test`.
// The Cloudflare Worker glue (HTTP fetches + KV cache + the fetch event) is
// gated to the wasm32 target.
//

pub mod error;
pub mod model;
pub mod render;
pub mod taphunter;
pub mod untappd;

// ---------------------------------------------------------------------------
// Cloudflare Worker glue (wasm32 only).
// ---------------------------------------------------------------------------
#[cfg(target_arch = "wasm32")]
mod worker_glue {
    use crate::error::{AppError, AppResult};
    use crate::model::{sort_rated, BeerEntry, RatedBeer};
    use crate::{render, taphunter, untappd};
    use futures::stream::{self, StreamExt};
    use worker::kv::KvStore;
    use worker::{
        console_log, event, Context, Env, Fetch, Headers, Method, Request, RequestInit, Response,
        Router,
    };

    const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";
    const CONCURRENT_REQUESTS: usize = 5;
    /// Cache successful ratings for a week.
    const CACHE_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
    /// Cache confirmed "no rating found" for a day (cheaper to recheck).
    const NOT_FOUND_TTL_SECONDS: u64 = 24 * 60 * 60;
    /// Bump this to invalidate all cached entries at once (e.g. after a parser
    /// change or to flush the poisoned "N/A" entries from the outage).
    /// v3: unrated-but-existing beers are now cached as a linked "N/A".
    /// v4: fuzzy Algolia matching (ignorePlurals + lastWords) changes results.
    /// v5: dropped lastWords (it force-matched beers not really on Untappd).
    const CACHE_VERSION: &str = "v5";

    fn cache_key(brewery: &str, name: &str) -> String {
        format!(
            "rating:{}:{}:{}",
            CACHE_VERSION,
            brewery.to_lowercase(),
            name.to_lowercase()
        )
    }

    /// GET a URL with our browser-ish User-Agent and return the body text.
    async fn fetch_text(url: &str) -> AppResult<String> {
        let headers = Headers::new();
        headers
            .set("User-Agent", USER_AGENT)
            .map_err(|e| AppError::Client(format!("Failed to set headers: {e}")))?;
        let req = Request::new_with_init(
            url,
            &RequestInit {
                method: Method::Get,
                headers,
                ..Default::default()
            },
        )
        .map_err(|e| AppError::Client(format!("Failed to create request: {e}")))?;
        let mut resp = Fetch::Request(req)
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Failed to get response: {e}")))?;
        resp.text()
            .await
            .map_err(|e| AppError::Network(format!("Failed to read response: {e}")))
    }

    /// POST an Algolia query and return the response body.
    async fn algolia_post(q: &untappd::AlgoliaQuery) -> AppResult<String> {
        let headers = Headers::new();
        headers
            .set("X-Algolia-Application-Id", &q.app_id)
            .and_then(|_| headers.set("X-Algolia-API-Key", &q.api_key))
            .and_then(|_| headers.set("Content-Type", "application/json"))
            .map_err(|e| AppError::Client(format!("Failed to set headers: {e}")))?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(q.body.clone().into()));
        let req = Request::new_with_init(&q.url, &init)
            .map_err(|e| AppError::Client(format!("Failed to create request: {e}")))?;
        let mut resp = Fetch::Request(req)
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Failed to get response: {e}")))?;
        resp.text()
            .await
            .map_err(|e| AppError::Network(format!("Failed to read response: {e}")))
    }

    /// Resolve a single beer's rating cell ("N/A" when unresolved).
    async fn resolve_rating(brewery: &str, name: &str) -> AppResult<crate::model::RatingResult> {
        let search = format!("{brewery} {name}");
        let q = untappd::build_query(&search);
        let body = algolia_post(&q).await?;
        untappd::parse_rating(&body)
    }

    /// Fetch ratings for all entries concurrently, using KV as a cache.
    /// Only successful ratings and confirmed not-founds are cached; transient
    /// failures (network/blocked) are never cached so they self-heal.
    async fn fetch_ratings(entries: &[BeerEntry], kv: &KvStore) -> Vec<String> {
        let results: Vec<(usize, String)> = stream::iter(entries.iter().enumerate())
            .map(|(idx, entry)| async move {
                let key = cache_key(&entry.brewery, &entry.name);

                if let Ok(Some(cached)) = kv.get(&key).text().await {
                    return (idx, cached);
                }

                let (cell, ttl) = match resolve_rating(&entry.brewery, &entry.name).await {
                    Ok(rating) => {
                        // Existing-but-unrated beers still link, but show "N/A";
                        // recheck them daily so a real rating appears sooner.
                        let ttl = if rating.rating == "N/A" {
                            NOT_FOUND_TTL_SECONDS
                        } else {
                            CACHE_TTL_SECONDS
                        };
                        (rating.to_cell(), Some(ttl))
                    }
                    Err(AppError::NotFound) => ("N/A".to_string(), Some(NOT_FOUND_TTL_SECONDS)),
                    Err(e @ AppError::Blocked(_)) => {
                        console_log!(
                            "Rating blocked for '{} {}': {}",
                            entry.brewery,
                            entry.name,
                            e
                        );
                        ("N/A".to_string(), None) // never cache a block
                    }
                    Err(e) => {
                        console_log!("Rating error for '{} {}': {}", entry.brewery, entry.name, e);
                        ("N/A".to_string(), None) // never cache transient failures
                    }
                };

                if let Some(ttl) = ttl {
                    if let Ok(put) = kv.put(&key, cell.clone()) {
                        if let Err(e) = put.expiration_ttl(ttl).execute().await {
                            console_log!("Failed to cache rating for key '{}': {}", key, e);
                        }
                    }
                }

                (idx, cell)
            })
            .buffer_unordered(CONCURRENT_REQUESTS)
            .collect()
            .await;

        let mut cells = vec!["N/A".to_string(); entries.len()];
        for (idx, cell) in results {
            cells[idx] = cell;
        }
        cells
    }

    async fn build_menu_html(kv: &KvStore) -> AppResult<String> {
        // 1. Resolve the TapHunter JSON endpoint, then the menu.
        let bigscreen = fetch_text(&taphunter::bigscreen_url()).await?;
        let json_url = taphunter::parse_json_url(&bigscreen)?;
        let menu_json = fetch_text(&json_url).await?;
        let entries = taphunter::parse_menu(&menu_json)?;

        // 2. Cross-reference Untappd ratings (cached).
        let ratings = fetch_ratings(&entries, kv).await;

        // 3. Pair, sort, render.
        let mut rated: Vec<RatedBeer> = entries
            .into_iter()
            .zip(ratings)
            .map(|(entry, rating_html)| RatedBeer { entry, rating_html })
            .collect();
        sort_rated(&mut rated);
        Ok(render::render(&rated))
    }

    #[event(fetch)]
    async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response, worker::Error> {
        Router::new()
            .get_async("/", |_req, ctx| async move {
                let html = async {
                    let kv = ctx
                        .kv("b30")
                        .map_err(|e| AppError::Client(format!("Failed to get KV store: {e}")))?;
                    build_menu_html(&kv).await
                }
                .await
                .map_err(worker::Error::from)?;
                Response::from_html(html)
            })
            .run(req, env)
            .await
    }
}
