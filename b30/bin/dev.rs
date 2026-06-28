//
// Native dev runner — exercise the full pipeline (or a single rating) against
// live sites without deploying to Cloudflare. Built only with `--features native`.
//
//   cargo run --features native --bin b30-dev -- menu [out.html]
//   cargo run --features native --bin b30-dev -- rating "Sierra Nevada Pale Ale"
//   cargo run --features native --bin b30-dev -- refresh-fixtures
//
// It reuses the exact same pure parsers/renderer the worker uses; only the HTTP
// (reqwest) differs from the wasm `worker::Fetch` path.
//

use lib::error::{AppError, AppResult};
use lib::model::{sort_rated, RatedBeer};
use lib::{render, taphunter, untappd};
use std::fs;
use std::io::Write;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("failed to build HTTP client")
}

fn get_text(c: &reqwest::blocking::Client, url: &str) -> AppResult<String> {
    c.get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| AppError::Network(format!("GET {url} failed: {e}")))
}

fn algolia_query(c: &reqwest::blocking::Client, search: &str) -> AppResult<String> {
    let q = untappd::build_query(search);
    c.post(&q.url)
        .header("X-Algolia-Application-Id", &q.app_id)
        .header("X-Algolia-API-Key", &q.api_key)
        .header("Content-Type", "application/json")
        .body(q.body)
        .send()
        .and_then(|r| r.text())
        .map_err(|e| AppError::Network(format!("Algolia query failed: {e}")))
}

fn cmd_rating(search: &str) -> AppResult<()> {
    let c = client();
    let body = algolia_query(&c, search)?;
    match untappd::parse_rating(&body) {
        Ok(r) => println!("rating={}  url={}", r.rating, r.url),
        Err(e) => println!("no rating: {e}"),
    }
    Ok(())
}

fn cmd_menu(out: Option<&str>) -> AppResult<()> {
    let c = client();
    let bigscreen = get_text(&c, &taphunter::bigscreen_url())?;
    let json_url = taphunter::parse_json_url(&bigscreen)?;
    let menu_json = get_text(&c, &json_url)?;
    let entries = taphunter::parse_menu(&menu_json)?;
    eprintln!("parsed {} taps", entries.len());

    let mut rated: Vec<RatedBeer> = Vec::with_capacity(entries.len());
    for entry in entries {
        let search = format!("{} {}", entry.brewery, entry.name);
        let rating_html = match algolia_query(&c, &search).and_then(|b| untappd::parse_rating(&b)) {
            Ok(r) => r.to_cell(),
            Err(e) => {
                eprintln!("  {search} -> N/A ({e})");
                "N/A".to_string()
            }
        };
        rated.push(RatedBeer { entry, rating_html });
    }
    sort_rated(&mut rated);
    let html = render::render(&rated);

    let resolved = rated.iter().filter(|b| b.rating_html != "N/A").count();
    eprintln!("resolved {}/{} ratings", resolved, rated.len());

    match out {
        Some(path) => {
            fs::write(path, &html)
                .map_err(|e| AppError::Internal(format!("write {path} failed: {e}")))?;
            eprintln!("wrote {path}");
        }
        None => {
            std::io::stdout().write_all(html.as_bytes()).ok();
        }
    }
    Ok(())
}

fn cmd_refresh_fixtures() -> AppResult<()> {
    let c = client();
    let dir = "b30/fixtures";

    let bigscreen = get_text(&c, &taphunter::bigscreen_url())?;
    fs::write(format!("{dir}/taphunter_bigscreen.html"), &bigscreen).ok();
    let json_url = taphunter::parse_json_url(&bigscreen)?;
    let menu = get_text(&c, &json_url)?;
    fs::write(format!("{dir}/taphunter_menu.json"), &menu).ok();

    let search = get_text(&c, &untappd::search_page_url("sierra nevada pale ale"))?;
    fs::write(format!("{dir}/untappd_search.html"), &search).ok();
    let algolia = algolia_query(&c, "sierra nevada pale ale")?;
    fs::write(format!("{dir}/algolia_beer_query.json"), &algolia).ok();
    let none = algolia_query(&c, "zzqxwv-not-a-real-beer-9999")?;
    fs::write(format!("{dir}/algolia_no_results.json"), &none).ok();

    eprintln!("refreshed fixtures in {dir}");
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("rating") => match args.get(2) {
            Some(q) => cmd_rating(q),
            None => Err(AppError::Client(
                "usage: rating \"<brewery> <name>\"".into(),
            )),
        },
        Some("menu") => cmd_menu(args.get(2).map(String::as_str)),
        Some("refresh-fixtures") => cmd_refresh_fixtures(),
        _ => Err(AppError::Client(
            "usage: b30-dev <menu [out.html] | rating \"<query>\" | refresh-fixtures>".into(),
        )),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
