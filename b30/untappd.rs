use reqwest;
use soup::prelude::*;
use std::error::Error;
use url::Url;

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
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
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