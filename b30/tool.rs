use clap::Parser;
use lib::*;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // Name of the beer to search for
    #[arg(index = 1)]
    beer_name: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    match args.beer_name {
        Some(name) => {
            let rating = get_beer_rating(&name).await;
            println!("Rating for '{}': {}", name, rating);
        },
        None => {
            let json_url = get_beerthirty_json().await;
            // println!("Menu JSON: {}", json_url);
            let df = b30_json_to_dataframe(&json_url).await;
            let df_html = dataframe_to_html(&df.unwrap());
            println!("{}", df_html.unwrap());
        }
    }
}
