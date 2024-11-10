use chrono::NaiveDateTime;
use futures::stream::{self, StreamExt};
use polars::prelude::*;
use regex::Regex;
use reqwest;
use serde_json::Value;
use soup::prelude::*;
use std::error::Error;
use url::Url;


static USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";


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
    untappd_rating: String,
}


pub async fn get_beerthirty_json() -> String {
    match get_beerthirty_json_internal().await {
        Ok(json_url) => json_url,
        Err(e) => {
            eprintln!("Error fetching Beer30 JSON URL: {}", e);
            "N/A".to_string()
        }
    }
}


async fn get_beerthirty_json_internal() -> Result<String, Box<dyn Error>> {
    // Fetch the main page
    let client = reqwest::Client::new();
    let response = client
        .get("http://www.taphunter.com/bigscreen/5469327503392768")
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("HTTP request returned status: {}", response.status()).into());
    }
    
    let html = response
        .text()
        .await
        .map_err(|e| format!("Failed to get response text: {}", e))?;
    
    let soup = Soup::new(&html);
    
    // Find all script tags
    let scripts = soup.tag("script").find_all();
    
    // Regex to find the getJSON URL
    let re = Regex::new(r#"getJSON\(['"](./)?json/([^'"]+)['"]"#)
        .map_err(|e| format!("Regex creation failed: {}", e))?;
    
    // Look through each script tag for the getJSON pattern
    for script in scripts {
        let content = script.text();
        if content.contains("getJSON") {
            if let Some(captures) = re.captures(&content) {
                // Get the relative path from the regex capture
                let relative_path = captures
                    .get(2)
                    .ok_or("Failed to capture JSON path")?
                    .as_str();
                
                // Construct the full URL
                return Ok(format!(
                    "http://www.taphunter.com/bigscreen/json/{}",
                    relative_path
                ));
            }
        }
    }
    
    Err("Could not find getJSON URL in any script tag".into())
}


pub async fn get_beer_rating(search_string: &str) -> String {
    match get_beer_rating_internal(search_string).await {
        Ok(rating_and_url) => rating_and_url,
        Err(e) => {
            eprintln!("Error fetching beer rating for '{}': {}", search_string, e);
            "N/A".to_string()
        }
    }
}


async fn get_beer_rating_internal(search_string: &str) -> Result<String, Box<dyn Error>> {
    let base_url = "https://untappd.com/search";
    let url = Url::parse_with_params(base_url, &[("q", search_string)])
        .map_err(|e| format!("URL parsing failed: {}", e))?;
    
    let client = reqwest::Client::new();
    let response = client
        .get(url.as_str())
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("HTTP request returned status: {}", response.status()).into());
    }
    
    let html = response
        .text()
        .await
        .map_err(|e| format!("Failed to get response text: {}", e))?;
    
    let soup = Soup::new(&html);
    
    // Find the first beer-item div
    let beer_item = soup
        .class("beer-item")
        .find()
        .ok_or("HTML parsing: Could not find beer-item div")?;
    
    // Find the first anchor tag within beer-item
    let anchor = beer_item
        .tag("a")
        .find()
        .ok_or("HTML parsing: Could not find anchor tag")?;
    
    // Get the href attribute
    let relative_url = anchor
        .get("href")
        .ok_or("HTML parsing: Could not find href attribute")?;
    
    // Find the caps div within the beer-item
    let caps_div = beer_item
        .class("caps")
        .find()
        .ok_or("HTML parsing: Could not find caps div")?;
    
    // Extract the data-rating attribute
    let rating = caps_div
        .get("data-rating")
        .ok_or("HTML parsing: Could not find data-rating attribute")?;
    
    // Construct the full URL
    let full_url = format!("https://untappd.com{}", relative_url);
    
    // Create the HTML anchor tag with the rating as text
    let result = format!("<a href=\"{}\">{}</a>", full_url, rating);
    
    Ok(result)
}


fn calculate_days_old(date_str: &str) -> Result<i64, Box<dyn Error>> {
    // Parse the date string into a NaiveDateTime
    let date = NaiveDateTime::parse_from_str(
        &format!("{} 00:00:00", date_str),
        "%m/%d/%Y %H:%M:%S"
    )?;
    
    // Get current time
    let now = chrono::Local::now().naive_local();
    
    // Calculate the difference in days
    let days_old = (now - date).num_days();
    
    Ok(days_old)
}


async fn fetch_untappd_ratings(entries: &[BeerEntry]) -> Result<Vec<String>, Box<dyn Error>> {
    const CONCURRENT_REQUESTS: usize = 5;
    
    // Create owned search strings with their indices
    let search_strings: Vec<(usize, String)> = entries.iter()
        .enumerate()
        .map(|(idx, entry)| (idx, format!("{} {}", entry.brewery, entry.name)))
        .collect();
    
    // Create a vector to store results with proper capacity
    let mut ratings = vec!["".to_string(); entries.len()];
    
    // Process requests while preserving order
    let results: Vec<(usize, String)> = stream::iter(search_strings)
        .map(|(idx, search_string)| async move {
            let rating = match get_beer_rating_internal(&search_string).await {
                Ok(rating) => rating,
                Err(e) => {
                    eprintln!("Error fetching rating for '{}': {}", search_string, e);
                    "N/A".to_string()
                }
            };
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


pub async fn b30_json_to_dataframe(url: &str) -> Result<DataFrame, Box<dyn Error>> {
    // Fetch JSON data
    let response = reqwest::get(url).await?.text().await?;
    let json: Vec<Value> = serde_json::from_str(&response)?;

    // Process each entry
    let mut entries = Vec::new();
    for item in json {
        let date_str = item["date_added"].as_str().unwrap_or("").to_string();
        let days_old = calculate_days_old(&date_str)?;

        let mut entry = BeerEntry {
            tap_number: item["serving_info"]["tap_number"].as_i64().unwrap_or(0) as i32,
            brewery: clean_text(&item["brewery"]["common_name"].as_str().unwrap_or("")),
            name: clean_text(&item["beer"]["beer_name"].as_str().unwrap_or("")),
            abv: clean_text(&item["beer"]["abv"].as_str().unwrap_or("")),
            category: clean_text(&item["beer"]["style_category"].as_str().unwrap_or("")),
            origin: clean_text(&item["brewery"]["origin"].as_str().unwrap_or("")),
            style: clean_text(&item["beer"]["style"].as_str().unwrap_or("")),
            days_old: days_old.to_string(),
            untappd_rating: String::new(), // Will be populated later
        };

        // Remove "**Nitro**" from brewery and name
        entry.brewery = entry.brewery.replace("**Nitro**", "").trim().to_string();
        entry.name = entry.name.replace("Nitro", "").replace("**Nitro**", "").trim().to_string();

        // Remove Brewery name from beer name (if duplicated).
        entry.name = entry.name.replace(&entry.brewery, "").trim().to_string();

        entries.push(entry);
    }

    // Create vectors for each column
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

    // Fetch all Untappd ratings concurrently
    let ratings = fetch_untappd_ratings(&entries).await?;

    // Create DataFrame
    let mut df = DataFrame::new(vec![
        Series::new("tap", tap_numbers),
        Series::new("brewery", breweries),
        Series::new("name", names),
        Series::new("abv", abvs),
        Series::new("category", categories),
        Series::new("origin", origins),
        Series::new("style", styles),
        Series::new("age", days_old),
        Series::new("untappd rating", ratings),
    ])?;

    df.sort_in_place(
        ["category", "abv"],
        false,
        true
    );

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
    }
    th { 
        background-color: #f2f2f2;
        font-weight: bold;
    }
    tr:nth-child(even) { 
        background-color: #f9f9f9;
    }
    tr:hover {
        background-color: #f5f5f5;
    }
  </style>
</head>
<body>
    "#);

    html.push_str("<table>\n<thead>\n<tr>");
    
    // Add headers
    for name in df.get_column_names() {
        html.push_str(&format!("<th>{}</th>", name));
    }
    html.push_str("</tr>\n</thead>\n<tbody>\n");

    // Add rows
    let height = df.height();
    for row in 0..height {
        html.push_str("<tr>");
        for col in df.get_columns() {
            let cell = col.get(row).unwrap();
            // Remove quotes from string values
            let cell_str = format!("{}", cell);
            let cleaned_value = if cell_str.starts_with('"') && cell_str.ends_with('"') {
                &cell_str[1..cell_str.len() - 1]
            } else {
                &cell_str
            };
            html.push_str(&format!("<td>{}</td>", cleaned_value));
        }
        html.push_str("</tr>\n");
    }

    html.push_str("</tbody>\n</table>");

    html.push_str("</body>");

    Ok(html)
}


fn clean_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .replace(" ,", ",")
        .trim()
        .to_string()
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

    #[test]
    fn test_url_extraction() {
        // Test the regex pattern with a sample script content
        let re = Regex::new(r#"getJSON\(['"](./)?json/([^'"]+)['"]"#).unwrap();
        let sample = r#"$.getJSON('./json/ahJzfnRoZXRhcGh1bnRlci1ocmRyHwsSEnRhcGh1bnRlcl9sb2NhdGlvbhiAgIDYsMrbCQw', function(beers) {"#;
        
        let captures = re.captures(sample).unwrap();
        let path = captures.get(2).unwrap().as_str();
        
        assert_eq!(
            path,
            "ahJzfnRoZXRhcGh1bnRlci1ocmRyHwsSEnRhcGh1bnRlcl9sb2NhdGlvbhiAgIDYsMrbCQw"
        );
    }

}
