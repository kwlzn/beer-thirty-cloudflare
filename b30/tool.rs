use clap::Parser;
use lib::{get_beer_rating, get_beerthirty_json};

/// Search for a beer's rating on Untappd
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Name of the beer to search for
    #[arg(index = 1)]
    beer_name: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let json_url = get_beerthirty_json().await;
    let rating = get_beer_rating(&args.beer_name).await;
    println!("Menu JSON: {}", json_url);
    println!("Rating for '{}': {}", args.beer_name, rating);
}
