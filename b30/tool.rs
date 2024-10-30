use clap::Parser;
use untappd::get_beer_rating;

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
        let rating = get_beer_rating(&args.beer_name).await;
    println!("Rating for '{}': {}", args.beer_name, rating);
}
