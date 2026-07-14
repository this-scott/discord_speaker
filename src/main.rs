use clap::Parser;
use directories::ProjectDirs;
use tokio_util::sync::CancellationToken;
use std::fs;
use std::path::Path;

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

    #[arg(short  = 'S', long, env = "SPOTIFY_CLIENT_SECRET")]
    spotify_secret: String,

    #[arg(short, long, default_value_t = default_cache_location())]
    cache_location: String,

    #[arg(short, long, env = "REDIRECT_URI")]
    redirect_uri: String,

    #[arg(short, long, env = "BIND_ADDR")]
    bind_addr: String,

    /// Wipe user cache
    #[arg(short = 'n', long)]
    new: bool,
}

#[tokio::main]
async fn main() {
    println!("Starting discord_speaker");

    // rustls decides tls framework
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let args = Args::parse();

    // read discord bot token, spotify app token, and .cache ALL IN ONE PLACE
    // clap guarantees these are present (flag or env var) or exits before we get here.
    let discord_token = args.discord_token;
    let spotify_token = args.spotify_token;
    let spotify_secret = args.spotify_secret;
    let cache_location = args.cache_location;
    let redirect_uri = args.redirect_uri;
    let bind_addr = args.bind_addr;

    let cache_path = Path::new(&cache_location);
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent).expect("failed to create cache directory");
    }

    // --new wipes the existing cache db so it's rebuilt with the current schema
    if args.new && cache_path.exists() {
        fs::remove_file(cache_path).expect("failed to remove existing cache");
        println!("Removed existing cache at {cache_location}");
    }

    // create auth handler in main and layer db under it
    let auth = auth::AuthHandler::new(spotify_token.clone(), spotify_secret, redirect_uri.clone(), cache_location)
        .await
        .expect("failed to open database");

    // Oauth callback listener. Bind on the main task so a port conflict fails fast
    let callback_router = auth.router();
    let listener = tokio::net::TcpListener::bind(bind_addr.clone())
        .await
        .unwrap_or_else(|err| panic!("failed to bind callback listener on {}: {}", bind_addr, err));
    tokio::spawn(async move {
        axum::serve(listener, callback_router)
            .await
            .expect("callback server crashed");
    });

    let cancel_token = CancellationToken::new();
    let cloned_token: CancellationToken = cancel_token.clone();

    // Spawn close signal handler, lives in background
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("Shutting down...");
        cloned_token.cancel();
    });
    
    discord::run_service(discord_token, spotify_token, auth, redirect_uri, cancel_token).await;
    println!("Shutting down bot");
}
