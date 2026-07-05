use std::collections::HashMap;
use std::sync::Arc;

use poise::serenity_prelude as serenity;
use librespot_connect::Spirc;
use rand::RngExt;
use songbird::SerenityInit;
use songbird::input::core::io::{MediaSourceStream, ReadOnlySource};
use songbird::input::{Input, RawAdapter};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use rand::{distr::Alphanumeric, Rng};


use crate::{auth, spotify};


/// Live Spirc handles keyed by guild, so end_speaker can stop the device that speaker started.
type SpircSessions = Arc<Mutex<HashMap<serenity::GuildId, Spirc>>>;

// TEMP DIAGNOSTIC: finding play errors the exact PlayError songbird hits while readying our audio input.
struct TrackErrLogger;

#[serenity::async_trait]
impl songbird::EventHandler for TrackErrLogger {
    async fn act(&self, ctx: &songbird::EventContext<'_>) -> Option<songbird::Event> {
        if let songbird::EventContext::Track(states) = ctx {
            for (state, _) in *states {
                eprintln!("[diag] songbird track state: {:?}", state.playing);
            }
        }
        None
    }
}

// poise command types
struct Data {
    client_id: String,
    ah: auth::AuthHandler,
    redirect_uri: String,
    spirc_sessions: SpircSessions
} // User data, stored and accessible in all command invocations
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

pub async fn run_service(discord_token: String, client_id: String, auth: auth::AuthHandler, redirect_uri: String, cancel_token: CancellationToken) {
    let intents = serenity::GatewayIntents::non_privileged() | serenity::GatewayIntents::MESSAGE_CONTENT;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![speaker(), end_speaker()],
            ..Default::default()
        })
        // Found this in the poise docs. I think it's passing user data to the commands but damn it's weird
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data { client_id: client_id, ah: auth, redirect_uri: redirect_uri, spirc_sessions: Arc::new(Mutex::new(HashMap::new()))})
            })
        })
        .build();

    let mut client = serenity::ClientBuilder::new(discord_token, intents)
        .framework(framework)
        .register_songbird()
        .await
        .unwrap();

    tokio::select! {
        _ = cancel_token.cancelled() => {
            // Perform library-specific graceful cleanup here
            println!("Cleaning up discord internal state...");
            client.shard_manager.shutdown_all().await;
        }
        result = client.start() => {
            if let Err(why) = result {
                eprintln!("Discord client error: {why:?}");
            }
        }
    }
}
// lowkey gonna throw the commands here

#[poise::command(slash_command)]
async fn speaker(
    ctx: Context<'_>
) -> Result<(), Error> {
    let data = ctx.data();
    let guild_id = ctx.guild_id().unwrap();
    let channel_id = ctx.guild().unwrap().voice_states
        .get(&ctx.author().id)
        .and_then(|vs| vs.channel_id)
        .ok_or("You must be in a voice channel")?;


    // //(NEXT): setup spotify auth

    let token = match data.ah.get_user_credential(ctx.author()).await {
        Ok(Some(token)) => token,
        Ok(None) => {
            ctx.say("No login credential found. Check dms for a spotify link").await?;
            
            let state = rand::rng().sample_iter(&Alphanumeric).take(16).map(char::from).collect();

            let link = format!("https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope=streaming&redirect_uri={}&state={}",data.client_id, data.redirect_uri, state);
            // create shared state id and send the link
            ctx.author().direct_message(ctx, poise::serenity_prelude::CreateMessage::new().content(format!("Authorize with this link {link}"))).await?;

            match data.ah.create_token_request(&state, ctx.author()).await {
                Ok(token) => token,
                Err(e) => {
                    eprintln!("Auth creation error: {e:?}");
                    ctx.say("No auth credential created").await?;
                    return Ok(());
                }
            }
        }
        Err(e) => {
            eprintln!("DB error fetching credential: {e:?}");
            ctx.say("Something went wrong. Try again later.").await?;
            return Ok(());
        }
    };

    ctx.say(format!("Attempting to join channel: {:?}", channel_id)).await?;

    //create a 'context' or instance of the spawned bot
    let sb_context = songbird::get(ctx.serenity_context())
        .await
        .unwrap();

    //test channel join
    match sb_context.join(guild_id, channel_id).await {
        Ok(_) => {
            ctx.say("Successfully joined channel!").await?;
        }
        Err(e) => {
            ctx.say(format!("Failed to join: {:?}", e)).await?;
            return Ok(());
        }
    }

    // check current contexts guild and creates a guard 
    if let Some(handler_lock) = sb_context.get(guild_id) {
        let mut handler = handler_lock.lock().await;

        // I think I might need to sit this and stream in poise data so end_speaker commands can work but let's try this for now
        let cancel_token = CancellationToken::new();
        let cloned_token: CancellationToken = cancel_token.clone();

        let (spirc, stream) = spotify::play_stream(token, cancel_token).await.unwrap();

        // hold the handle so end_speaker can later stop/disconnect this guild's device
        data.spirc_sessions.lock().await.insert(guild_id, spirc);

        // MSS only accepts sync sources
        let mss = MediaSourceStream::new(Box::new(ReadOnlySource::new(stream)), Default::default());
        let source = RawAdapter::new(mss, 44100, 2);


        let input = Input::from(source);
        let track = handler.play_input(input);
        // TEMP DIAGNOSTIC: print the real PlayError when songbird errors the track during prepare.
        let _ = track.add_event(
            songbird::Event::Track(songbird::TrackEvent::Error),
            TrackErrLogger,
        );

    } else {
        ctx.say("Error: Could not get voice handler after joining!").await?;
    }

    Ok(())
}

#[poise::command(slash_command)]
async fn end_speaker(
    ctx: Context<'_>
) -> Result<(), Error> {
    let data = ctx.data();
    let guild_id = ctx.guild_id().ok_or("This command must be used in a guild")?;

    // pull the handle for this guild; if there isn't one, nothing is playing
    let spirc = match data.spirc_sessions.lock().await.remove(&guild_id) {
        Some(spirc) => spirc,
        None => {
            ctx.say("Nothing is playing here.").await?;
            return Ok(());
        }
    };

    // TODO: stop/disconnect the Spirc device (e.g. spirc.shutdown()) using `spirc`
    // TODO: leave the voice channel via songbird
    let _ = spirc;

    Ok(())
}