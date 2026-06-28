//
// Plain data models shared by the parsing, rendering, and worker layers.
//

/// One beer on tap, as parsed from the TapHunter menu JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct BeerEntry {
    pub tap_number: i32,
    pub brewery: String,
    pub name: String,
    pub abv: String,
    pub category: String,
    pub origin: String,
    pub style: String,
    pub days_old: i32,
}

/// A rating + review link resolved from Untappd (via Algolia).
#[derive(Debug, Clone, PartialEq)]
pub struct RatingResult {
    /// Rating score formatted to two decimals, e.g. "3.62".
    pub rating: String,
    /// Absolute URL to the beer's Untappd page.
    pub url: String,
}

impl RatingResult {
    /// Render the rating as the anchor cell used in the output table.
    pub fn to_cell(&self) -> String {
        format!("<a href=\"{}\">{}</a>", self.url, self.rating)
    }
}

/// A beer entry paired with its pre-rendered rating cell ("N/A" if unresolved).
#[derive(Debug, Clone)]
pub struct RatedBeer {
    pub entry: BeerEntry,
    pub rating_html: String,
}

impl BeerEntry {
    /// ABV parsed as a number for sorting/heatmapping; 0.0 if unparseable.
    pub fn abv_value(&self) -> f64 {
        self.abv
            .replace('%', "")
            .trim()
            .parse::<f64>()
            .unwrap_or(0.0)
    }
}

/// Sort rated beers by category (ascending), then by ABV ascending within a
/// category. (The previous implementation sorted ABV lexicographically via a
/// string column — this sorts numerically, which matches the stated intent.)
pub fn sort_rated(beers: &mut [RatedBeer]) {
    beers.sort_by(|a, b| {
        a.entry.category.cmp(&b.entry.category).then(
            a.entry
                .abv_value()
                .partial_cmp(&b.entry.abv_value())
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
}
