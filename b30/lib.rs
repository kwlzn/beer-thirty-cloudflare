mod scraper;

use chrono::NaiveDateTime;
use futures::stream::{self, StreamExt};
use polars_core::prelude::*;
use regex::Regex;
use serde_json::Value;
use url::Url;
use worker::{console_log, event, Context, Env, Fetch, Headers, Method, Request, Response, Router};
use worker_kv::KvStore;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";
const CONCURRENT_REQUESTS: u32 = 5;
const BASE_TAPHUNTER_URL: &str = "http://www.taphunter.com/bigscreen";
const BASE_UNTAPPD_URL: &str = "https://untappd.com";
const CACHE_TTL_SECONDS: u64 = 7 * 24 * 60 * 60; // 1 week

#[derive(Debug)]
pub enum AppError {
    Client(String),
    Network(String),
    Parse(String),
    Internal(String),
}

impl std::error::Error for AppError {}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Client(msg) => write!(f, "Client error: {}", msg),
            AppError::Network(msg) => write!(f, "Network error: {}", msg),
            AppError::Parse(msg) => write!(f, "Parse error: {}", msg),
            AppError::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl From<AppError> for worker::Error {
    fn from(error: AppError) -> Self {
        worker::Error::from(error.to_string())
    }
}

type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone)]
struct BeerEntry {
    tap_number: i32,
    brewery: String,
    name: String,
    abv: String,
    category: String,
    origin: String,
    style: String,
    days_old: i32,
}

fn clean_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .replace(" ,", ",")
        .trim()
        .to_string()
}

fn calculate_days_old(date_str: &str) -> AppResult<i64> {
    NaiveDateTime::parse_from_str(&format!("{} 00:00:00", date_str), "%m/%d/%Y %H:%M:%S")
        .map_err(|e| AppError::Parse(format!("Failed to parse date: {}", e)))
        .map(|date| {
            let now = chrono::Local::now().naive_local();
            (now - date).num_days()
        })
}

fn generate_cache_key(brewery: &str, name: &str) -> String {
    format!("rating:{}:{}", brewery.to_lowercase(), name.to_lowercase())
}

pub async fn get_beerthirty_json() -> String {
    match get_beerthirty_json_internal().await {
        Ok(json_url) => json_url,
        Err(e) => {
            console_log!("Error fetching Beer30 JSON URL: {}", e);
            "N/A".to_string()
        }
    }
}

async fn get_beerthirty_json_internal() -> AppResult<String> {
    // Fetch the main menu page.
    let mut headers = Headers::new();
    headers.set("User-Agent", USER_AGENT)
        .map_err(|e| AppError::Client(format!("Failed to set headers: {}", e)))?;

    let req = Request::new_with_init(
        &format!("{}/5469327503392768", BASE_TAPHUNTER_URL),
        &worker::RequestInit {
            method: Method::Get,
            headers,
            ..Default::default()
        },
    )
    .map_err(|e| AppError::Client(format!("Failed to create request: {}", e)))?;

    let mut resp = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Failed to get response: {}", e)))?;
    let html = resp.text().await
        .map_err(|e| AppError::Network(format!("Failed to get response text: {}", e)))?;

    let re = Regex::new(r#"getJSON\(['"](./)?json/([^'"]+)['"]"#)
        .map_err(|e| AppError::Parse(format!("Regex creation failed: {}", e)))?;

    // Parse the getJSON fetch via regex.
    if let Some(captures) = re.captures(&html) {
        let relative_path = captures
            .get(2)
            .ok_or_else(|| AppError::Parse("Failed to capture JSON path".into()))?
            .as_str();

        Ok(format!("{}/json/{}", BASE_TAPHUNTER_URL, relative_path))
    } else {
        Err(AppError::Parse("Could not find getJSON URL".into()))
    }
}

pub async fn get_beer_rating(search_string: &str) -> String {
    match get_beer_rating_internal(search_string).await {
        Ok(rating_and_url) => rating_and_url,
        Err(e) => {
            console_log!("Error fetching beer rating for '{}': {}", search_string, e);
            "N/A".to_string()
        }
    }
}

async fn get_beer_rating_internal(search_string: &str) -> AppResult<String> {
    let url = Url::parse_with_params(
        &format!("{}/search", BASE_UNTAPPD_URL),
        &[("q", search_string)],
    )
    .map_err(|e| AppError::Parse(format!("Failed to parse URL: {}", e)))?;

    let mut headers = Headers::new();
    headers.set("User-Agent", USER_AGENT)
        .map_err(|e| AppError::Client(format!("Failed to set headers: {}", e)))?;

    let req = Request::new_with_init(
        url.as_str(),
        &worker::RequestInit {
            method: Method::Get,
            headers,
            ..Default::default()
        },
    )
    .map_err(|e| AppError::Client(format!("Failed to create request: {}", e)))?;

    let mut resp = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Failed to get response: {}", e)))?;
    let html = resp.text().await
        .map_err(|e| AppError::Network(format!("Failed to get response text: {}", e)))?;

    // Find the first beer-item div
    let beer_items = scraper::find_elements_by_class(&html, "beer-item");
    let beer_item = beer_items
        .first()
        .ok_or_else(|| AppError::Parse("Could not find beer-item div".into()))?;

    // Find the first anchor tag within beer-item
    let anchor = scraper::find_first_anchor(beer_item.get_content())
        .ok_or_else(|| AppError::Parse("Could not find anchor tag".into()))?;

    // Get the href attribute
    let relative_url = anchor
        .get_attr("href")
        .ok_or_else(|| AppError::Parse("Could not find href attribute".into()))?;

    // Find the caps div within the beer-item
    let caps_divs = scraper::find_elements_by_class(beer_item.get_content(), "caps");
    let caps = caps_divs
        .first()
        .ok_or_else(|| AppError::Parse("Could not find caps div".into()))?;

    // Extract the data-rating attribute
    let rating = caps
        .get_attr("data-rating")
        .ok_or_else(|| AppError::Parse("Could not find data-rating attribute".into()))?;

    Ok(format!(
        "<a href=\"{}{}\">{}</a>",
        BASE_UNTAPPD_URL, relative_url, rating
    ))
}

async fn fetch_untappd_ratings(
    entries: &[BeerEntry],
    kv: &KvStore,
) -> AppResult<Vec<String>> {
    let mut ratings = vec!["".to_string(); entries.len()];

    // Process entries concurrently while preserving order
    let results: Vec<(usize, String)> = stream::iter(entries.iter().enumerate())
        .map(|(idx, entry)| async move {
            let cache_key = generate_cache_key(&entry.brewery, &entry.name);

            // Try to get from cache first
            if let Ok(Some(cached_rating)) = kv.get(&cache_key).text().await {
                return (idx, cached_rating);
            }

            // If not in cache, fetch from Untappd
            let search_string = format!("{} {}", entry.brewery, entry.name);
            let rating = match get_beer_rating_internal(&search_string).await {
                Ok(rating) => rating,
                Err(e) => {
                    console_log!("Error fetching rating for '{}': {}", search_string, e);
                    "N/A".to_string()
                }
            };

            // Store in cache with TTL - including non-existent results
            if let Err(e) = kv
                .put(&cache_key, rating.clone())
                .expect("Failed to create PUT object")
                .expiration_ttl(CACHE_TTL_SECONDS)
                .execute()
                .await
            {
                console_log!("Failed to cache rating for '{}': {}", search_string, e);
            }

            (idx, rating)
        })
        .buffer_unordered(CONCURRENT_REQUESTS as usize)
        .collect()
        .await;

    // Place results in the correct positions
    for (idx, rating) in results {
        ratings[idx] = rating;
    }

    Ok(ratings)
}

pub async fn b30_json_to_dataframe(url: &str, kv: &KvStore) -> AppResult<DataFrame> {
    // Fetch JSON data.
    let mut headers = Headers::new();
    headers.set("User-Agent", USER_AGENT)
        .map_err(|e| AppError::Client(format!("Failed to set headers: {}", e)))?;

    let req = Request::new_with_init(
        url,
        &worker::RequestInit {
            method: Method::Get,
            headers,
            ..Default::default()
        },
    )
    .map_err(|e| AppError::Client(format!("Failed to create request: {}", e)))?;

    let mut resp = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Failed to get response: {}", e)))?;
    let text = resp.text().await
        .map_err(|e| AppError::Network(format!("Failed to get response text: {}", e)))?;
    let json: Vec<Value> = serde_json::from_str(&text)
        .map_err(|e| AppError::Parse(format!("Failed to parse JSON: {}", e)))?;

    // Process each entry.
    let mut entries = Vec::new();
    for item in json {
        let date_str = item["date_added"].as_str().unwrap_or("").to_string();
        let days_old = calculate_days_old(&date_str)?;

        let mut entry = BeerEntry {
            tap_number: item["serving_info"]["tap_number"].as_i64().unwrap_or(0) as i32,
            brewery: clean_text(&item["brewery"]["common_name"].as_str().unwrap_or("")),
            name: clean_text(&item["beer"]["beer_name"].as_str().unwrap_or("")),
            abv: {
                let abv = clean_text(&item["beer"]["abv"].as_str().unwrap_or(""));
                // Convert empty ABV to "0.0".
                if abv.is_empty() {
                    "0.0".to_string()
                } else {
                    abv
                }
            },
            category: clean_text(&item["beer"]["style_category"].as_str().unwrap_or("")),
            origin: clean_text(&item["brewery"]["origin"].as_str().unwrap_or("")),
            style: clean_text(&item["beer"]["style"].as_str().unwrap_or("")),
            days_old: days_old as i32,
        };

        // Remove extraneous "**Nitro**" from brewery and name (used to indicate nitro taps).
        entry.brewery = entry.brewery.replace("**Nitro**", "").trim().to_string();
        entry.name = entry
            .name
            .replace("**NITRO**", "")
            .replace("**Nitro**", "")
            .replace("NITRO", "")
            .replace("Nitro", "")
            .trim()
            .to_string();

        // Remove Brewery name from beer name (if duplicated).
        entry.name = entry.name.replace(&entry.brewery, "").trim().to_string();
        entries.push(entry);
    }

    // Create vectors for each column.
    let tap_numbers: Vec<i32> = entries.iter().map(|e| e.tap_number).collect();
    let breweries: Vec<String> = entries.iter().map(|e| e.brewery.clone()).collect();
    let names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    let abvs: Vec<String> = entries.iter().map(|e| e.abv.clone()).collect();
    let categories: Vec<String> = entries.iter().map(|e| e.category.clone()).collect();
    let origins: Vec<String> = entries.iter().map(|e| e.origin.clone()).collect();
    let styles: Vec<String> = entries.iter().map(|e| e.style.clone()).collect();
    let days_old: Vec<i32> = entries.iter().map(|e| e.days_old).collect();

    // Fetch all Untappd ratings concurrently.
    let ratings = fetch_untappd_ratings(&entries, kv).await?;

    // Create DataFrame.
    let mut df = DataFrame::new(vec![
        Series::new("category", categories),
        Series::new("tap", tap_numbers),
        Series::new("brewery", breweries),
        Series::new("name", names),
        Series::new("abv", abvs),
        Series::new("origin", origins),
        Series::new("style", styles),
        Series::new("age", days_old),
        Series::new("rating", ratings),
    ])
    .map_err(|e| AppError::Internal(format!("Failed to create DataFrame: {}", e)))?;

    df.sort_in_place(["category", "abv"], false, true)
        .map_err(|e| AppError::Internal(format!("Failed to sort DataFrame: {}", e)))?;

    Ok(df)
}

// Converts a Dataframe into an HTML string for output.
pub fn dataframe_to_html(df: &DataFrame) -> AppResult<String> {
    let mut html = String::from(
        r#"
<head>
  <style>
    table { 
        border-collapse: collapse; 
        width: 100%; 
        margin: 20px 0;
        font-family: Arial, sans-serif;
    }
    th, td { 
        border: 1px solid #ddd; 
        padding: 8px; 
        text-align: left;
        vertical-align: middle;
    }
    th { 
        background-color: #f2f2f2;
        font-weight: bold;
        text-align: center !important;  /* Force center alignment for all headers */
    }
    tr:nth-child(even) td:not(.category-cell) { 
        background-color: #f9f9f9;
    }
    tr:nth-child(odd) td:not(.category-cell) { 
        background-color: #ffffff;
    }
    tr:hover td:not(.category-cell) {
        background-color: #f5f5f5;
    }
    .category-cell {
        font-weight: bold;
        text-align: center;
    }
    .category-cell-even {
        background-color: #f0f6fc;
    }
    .category-cell-odd {
        background-color: #ffffff;
    }
    .numeric {
        text-align: right;
    }
    .abv-low {
        background-color: #1a9850 !important;
        color: black;
    }
    .abv-medium-low {
        background-color: #91cf60 !important;
        color: black;
    }
    .abv-medium {
        background-color: #fee08b !important;
        color: black;
    }
    .abv-high {
        background-color: #fc8d59 !important;
        color: black;
    }
  </style>
</head>
<body>
"#,
    );

    html.push_str("<table>\n<thead>\n<tr>");

    // Add headers
    for name in df.get_column_names() {
        html.push_str(&format!("<th>{}</th>", name));
    }
    html.push_str("</tr>\n</thead>\n<tbody>\n");

    let abv_idx = df
        .get_column_names()
        .iter()
        .position(|&name| name == "abv")
        .ok_or_else(|| AppError::Internal("ABV column not found".into()))?;
    let category_idx = df
        .get_column_names()
        .iter()
        .position(|&name| name == "category")
        .ok_or_else(|| AppError::Internal("Category column not found".into()))?;

    let height = df.height();
    let mut current_category = String::new();
    let mut category_number = 0;

    for row in 0..height {
        let mut row_started = false;

        for (col_idx, col) in df.get_columns().iter().enumerate() {
            let cell = col.get(row).unwrap();
            let cell_str = format!("{}", cell);
            let cleaned_value = if cell_str.starts_with('"') && cell_str.ends_with('"') {
                &cell_str[1..cell_str.len() - 1]
            } else {
                &cell_str
            };

            // Handle category column
            if col_idx == category_idx {
                let normalized_value = if cleaned_value.trim().is_empty() {
                    "(Uncategorized)"
                } else {
                    cleaned_value
                };

                if normalized_value != &current_category {
                    let mut count = 1;
                    for future_row in (row + 1)..height {
                        let future_cell = df.get_columns()[category_idx].get(future_row).unwrap();
                        let future_value = format!("{}", future_cell);
                        let future_cleaned =
                            if future_value.starts_with('"') && future_value.ends_with('"') {
                                &future_value[1..future_value.len() - 1]
                            } else {
                                &future_value
                            };
                        let future_normalized = if future_cleaned.trim().is_empty() {
                            "(Uncategorized)"
                        } else {
                            future_cleaned
                        };
                        if future_normalized == normalized_value {
                            count += 1;
                        } else {
                            break;
                        }
                    }

                    if !row_started {
                        html.push_str("<tr>");
                        row_started = true;
                    }

                    let category_class = if category_number % 2 == 0 {
                        "category-cell category-cell-even"
                    } else {
                        "category-cell category-cell-odd"
                    };

                    let display_value = if normalized_value == "(Uncategorized)" {
                        ""
                    } else {
                        normalized_value
                    };

                    html.push_str(&format!(
                        "<td class=\"{}\" rowspan=\"{}\">{}</td>",
                        category_class, count, display_value
                    ));

                    current_category = normalized_value.to_string();
                    category_number += 1;
                }
            } else {
                if !row_started {
                    html.push_str("<tr>");
                    row_started = true;
                }

                let column_name = df.get_column_names()[col_idx];
                let is_numeric = matches!(column_name, "tap" | "age" | "days_old" | "rating");

                if col_idx == abv_idx {
                    let abv_value = cleaned_value.replace('%', "").parse::<f64>().unwrap_or(0.0);
                    let class_name = match abv_value {
                        x if x < 6.0 => "abv-low numeric",
                        x if x < 6.5 => "abv-medium-low numeric",
                        x if x < 7.0 => "abv-medium numeric",
                        _ => "abv-high numeric",
                    };
                    html.push_str(&format!(
                        "<td class=\"{}\">{}</td>",
                        class_name, cleaned_value
                    ));
                } else if is_numeric {
                    html.push_str(&format!("<td class=\"numeric\">{}</td>", cleaned_value));
                } else {
                    html.push_str(&format!("<td>{}</td>", cleaned_value));
                }
            }
        }

        if row_started {
            html.push_str("</tr>\n");
        }
    }

    html.push_str("</tbody>\n</table>");
    html.push_str("</body>");

    Ok(html)
}

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response, worker::Error> {
    let router = Router::new();
    Ok(router
        .get_async("/", |_req, ctx| async move {
            let result: Result<Response, worker::Error> = (|| async {
                let kv = ctx.kv("b30")
                    .map_err(|e| AppError::Client(format!("Failed to get KV store: {}", e)))?;
                let json_url = get_beerthirty_json().await;
                let df = b30_json_to_dataframe(&json_url, &kv).await?;
                let df_html = dataframe_to_html(&df)?;
                Response::from_html(format!("{}", df_html))
                    .map_err(|e| AppError::Internal(format!("Failed to create response: {}", e)))
            })()
            .await
            .map_err(worker::Error::from);

            result
        })
        .run(req, env)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_invalid_beer() {
        let result = get_beer_rating("ThisBeerDefinitelyDoesNotExist123456789").await;
        assert_eq!(result, "N/A");
    }

    #[tokio::test]
    async fn test_output_format() {
        let result = get_beer_rating("Sierra Nevada Pale Ale").await;
        assert!(
            result.starts_with("<a href=\"https://untappd.com/")
                && result.ends_with("</a>")
                && result.contains("\">")
        );
    }
}
