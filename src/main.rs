use clap::Parser;
use directories::ProjectDirs;
use tokio_util::sync::CancellationToken;

mod discord;
mod spotify;
mod auth;
mod db;

/// Default cache directory for this app, e.g. ~/.cache/discord_spotify on Linux.
fn default_cache_location() -> String {
    ProjectDirs::from("com", "scott", "discord_spotify")
        .expect("could not determine a home directory for the cache location")
        .cache_dir()
        .to_string_lossy()
        .into_owned()
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Discord Bot Token
    #[arg(short, long, env = "DISCORD_TOKEN")]
    discord_token: String,

    /// Spotify App Token (not to be confused with user oauth codes)
    #[arg(short, long, env = "SPOTIFY_CLIENT_ID")]
    spotify_token: String,

    #[arg(short, long, env = "SPOTIFY_CLIENT_SECRET")]
    spotify_secret: String,

    #[arg(short, long, default_value_t = default_cache_location())]
    cache_location: String
}

#[tokio::main]
async fn main() {
    println!("Hello, world!");

    let args = Args::parse();

    // read discord bot token, spotify app token, and .cache ALL IN ONE PLACE
    // clap guarantees these are present (flag or env var) or exits before we get here.
    let discord_token = args.discord_token;
    let spotify_token = args.spotify_token;
    let spotify_secret = args.spotify_secret;
    let cache_location = args.cache_location; 
    //todo: also create a simple credential handler
    // create auth handler in main and layer db under it
    let auth = auth::AuthHandler::new(spotify_token.clone(), spotify_secret, cache_location)
        .await
        .expect("failed to open database");

    let cancel_token = CancellationToken::new();
    let cloned_token = cancel_token.clone();

    // Spawn close signal handler, lives in background
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("Shutting down...");
        cloned_token.cancel();
    });
    
    discord::run_service(discord_token, spotify_token, auth, cancel_token).await;
    println!("Shutting down bot");
}
