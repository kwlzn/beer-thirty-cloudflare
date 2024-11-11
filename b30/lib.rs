mod scraper;

use chrono::NaiveDateTime;
use futures::stream::{self, StreamExt};
use polars_core::prelude::*;
use regex::Regex;
use serde_json::Value;
use std::error::Error;
use worker::{event, console_log, Context, Env, Fetch, Headers, Method, Request, Response, Router};
use worker_kv::{KvStore};
use url::Url;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";
const CONCURRENT_REQUESTS: usize = 5;
const BASE_TAPHUNTER_URL: &str = "http://www.taphunter.com/bigscreen";
const BASE_UNTAPPD_URL: &str = "https://untappd.com";
const CACHE_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;  // 1 week

#[derive(Debug)]
struct BeerEntry {
    tap_number: i32,
    brewery: String,
    name: String,
    abv: String,
    category: String,
    origin: String,
    style: String,
    days_old: String,
}

fn clean_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .replace(" ,", ",")
        .trim()
        .to_string()
}

fn calculate_days_old(date_str: &str) -> Result<i64, Box<dyn Error>> {
    let date = NaiveDateTime::parse_from_str(
        &format!("{} 00:00:00", date_str),
        "%m/%d/%Y %H:%M:%S"
    )?;

    let now = chrono::Local::now().naive_local();
    let days_old = (now - date).num_days();

    Ok(days_old)
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

async fn get_beerthirty_json_internal() -> Result<String, Box<dyn Error>> {
    // Fetch the main menu page.
    let mut headers = Headers::new();
    headers.set("User-Agent", USER_AGENT)?;
    
    let req = Request::new_with_init(
        &format!("{}/5469327503392768", BASE_TAPHUNTER_URL),
        &worker::RequestInit {
            method: Method::Get,
            headers,
            ..Default::default()
        },
    )?;
    
    let mut resp = Fetch::Request(req).send().await.map_err(|e| format!("Failed to get response text: {}", e))?;
    let html = resp.text().await?;
    let re = Regex::new(r#"getJSON\(['"](./)?json/([^'"]+)['"]"#).map_err(|e| format!("Regex creation failed: {}", e))?;
    
    // Parse the getJSON fetch via regex.
    if let Some(captures) = re.captures(&html) {
        let relative_path = captures
            .get(2)
            .ok_or("Failed to capture JSON path")?
            .as_str();

        return Ok(format!(
            "{}/json/{}",
            BASE_TAPHUNTER_URL,
            relative_path
        ));
    }
    
    Err("Could not find getJSON URL".into())
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

async fn get_beer_rating_internal(search_string: &str) -> Result<String, Box<dyn Error>> {
    let url = Url::parse_with_params(
        &format!("{}/search", BASE_UNTAPPD_URL),
        &[("q", search_string)]
    )?;

    let mut headers = Headers::new();
    headers.set("User-Agent", USER_AGENT)?;
    
    let req = Request::new_with_init(
        url.as_str(),
        &worker::RequestInit {
            method: Method::Get,
            headers,
            ..Default::default()
        },
    )?;

    let mut resp = Fetch::Request(req).send().await.map_err(|e| format!("Failed to get response text: {}", e))?;    
    let html = resp.text().await?;
    
    // Find the first beer-item div
    let beer_items = scraper::find_elements_by_class(&html, "beer-item");
    let beer_item = beer_items.first()
        .ok_or("HTML parsing: Could not find beer-item div")?;
    
    // Find the first anchor tag within beer-item
    let anchor = scraper::find_first_anchor(beer_item.get_content())
        .ok_or("HTML parsing: Could not find anchor tag")?;
    
    // Get the href attribute
    let relative_url = anchor
        .get_attr("href")
        .ok_or("HTML parsing: Could not find href attribute")?;
    
    // Find the caps div within the beer-item
    let caps_divs = scraper::find_elements_by_class(beer_item.get_content(), "caps");
    let caps = caps_divs.first()
        .ok_or("HTML parsing: Could not find caps div")?;
    
    // Extract the data-rating attribute
    let rating = caps
        .get_attr("data-rating")
        .ok_or("HTML parsing: Could not find data-rating attribute")?;
    
    Ok(format!(
        "<a href=\"{}{}\">{}</a>",
        BASE_UNTAPPD_URL,
        relative_url,
        rating
    ))
}

async fn fetch_untappd_ratings(entries: &[BeerEntry], kv: &KvStore) -> Result<Vec<String>, Box<dyn Error>> {
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
            if let Err(e) = kv.put(&cache_key, rating.clone())
                .expect("Failed to create PUT object")
                .expiration_ttl(CACHE_TTL_SECONDS)
                .execute()
                .await
            {
                console_log!("Failed to cache rating for '{}': {}", search_string, e);
            }

            (idx, rating)
        })
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect()
        .await;

    // Place results in the correct positions
    for (idx, rating) in results {
        ratings[idx] = rating;
    }

    Ok(ratings)
}

pub async fn b30_json_to_dataframe(url: &str, kv: &KvStore) -> Result<DataFrame, Box<dyn Error>> {
    // Fetch JSON data.
    let mut headers = Headers::new();
    headers.set("User-Agent", USER_AGENT)?;

    let req = Request::new_with_init(
        url,
        &worker::RequestInit {
            method: Method::Get,
            headers,
            ..Default::default()
        },
    )?;

    let mut resp = Fetch::Request(req).send().await.map_err(|e| format!("Failed to get response text: {}", e))?;
    let text = resp.text().await?;
    let json: Vec<Value> = serde_json::from_str(&text)?;

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
                if abv.is_empty() { "0.0".to_string() } else { abv }
            },
            category: clean_text(&item["beer"]["style_category"].as_str().unwrap_or("")),
            origin: clean_text(&item["brewery"]["origin"].as_str().unwrap_or("")),
            style: clean_text(&item["beer"]["style"].as_str().unwrap_or("")),
            days_old: days_old.to_string(),
        };

        // Remove extraneous "**Nitro**" from brewery and name (used to indicate nitro taps).
        entry.brewery = entry.brewery.replace("**Nitro**", "").trim().to_string();
        entry.name = entry.name
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
    let days_old: Vec<i64> = entries.iter()
        .map(|e| e.days_old.parse::<i64>().unwrap_or(0))
        .collect();

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
    ])?;

    df.sort_in_place(
        ["category", "abv"],
        false,
        true
    )?;

    Ok(df)
}

// Converts a Dataframe into an HTML string for output.
pub fn dataframe_to_html(df: &DataFrame) -> Result<String, Box<dyn Error>> {
    let mut html = String::from(r#"
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
"#);

    html.push_str("<table>\n<thead>\n<tr>");
    
    // Add headers - no longer need class for alignment since all headers are centered via CSS
    for name in df.get_column_names() {
        html.push_str(&format!("<th>{}</th>", name));
    }
    html.push_str("</tr>\n</thead>\n<tbody>\n");

    let abv_idx = df.get_column_names().iter().position(|&name| name == "abv")
        .ok_or("ABV column not found")?;
    let category_idx = df.get_column_names().iter().position(|&name| name == "category")
        .ok_or("Category column not found")?;

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
                        let future_cleaned = if future_value.starts_with('"') && future_value.ends_with('"') {
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
                        category_class,
                        count,
                        display_value
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
                let is_numeric = matches!(column_name, "tap" | "age" | "rating");

                if col_idx == abv_idx {
                    let abv_value = cleaned_value.replace('%', "").parse::<f64>().unwrap_or(0.0);
                    let class_name = match abv_value {
                        x if x < 6.0 => "abv-low numeric",
                        x if x < 6.5 => "abv-medium-low numeric",
                        x if x < 7.0 => "abv-medium numeric",
                        _ => "abv-high numeric",
                    };
                    html.push_str(&format!("<td class=\"{}\">{}</td>", class_name, cleaned_value));
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


// Cloudflare worker main fetch entrypoint.
#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response, Box<dyn Error>> {
    let router = Router::new();
    Ok(
        router
        .get_async("/b30", |_req, ctx| async move {
            let kv = ctx.kv("b30")?;
            let json_url = get_beerthirty_json().await;
            let df = b30_json_to_dataframe(&json_url, &kv).await;
            let df_html = dataframe_to_html(&df.unwrap());
            Ok(Response::from_html(format!("{}", df_html.unwrap()))?)
        })
        .run(req, env).await?
    )
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
            result.starts_with("<a href=\"https://untappd.com/") && 
            result.ends_with("</a>") && 
            result.contains("\">")
        );
    }
}
