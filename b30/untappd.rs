use reqwest;
use soup::prelude::*;
use std::error::Error;
use url::Url;

pub async fn get_beer_rating(search_string: &str) -> String {
    match get_beer_rating_internal(search_string).await {
        Ok(rating) => rating,
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
    
    let beer_item = soup
        .class("beer-item")
        .find()
        .ok_or("HTML parsing: Could not find beer-item div")?;
    
    let caps_div = beer_item
        .class("caps")
        .find()
        .ok_or("HTML parsing: Could not find caps div")?;
    
    let rating = caps_div
        .get("data-rating")
        .ok_or("HTML parsing: Could not find data-rating attribute")?;
    
    Ok(rating.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_invalid_beer() {
        let result = get_beer_rating("ThisBeerDefinitelyDoesNotExist123456789").await;
        assert_eq!(result, "N/A");
    }
}