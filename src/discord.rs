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
use rand::{distr::Alphanumeric};


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

type ActiveUsers = Arc<Mutex<HashMap<serenity::UserId, serenity::ChannelId>>>;

struct VoiceStateTracker {
    tracker: ActiveUsers,
    spirc_tracker: SpircSessions

}
// todo: follow user into new voice channels
#[serenity::async_trait]
impl serenity::EventHandler for VoiceStateTracker {
    async fn voice_state_update(&self, ctx: serenity::Context, old: Option<serenity::VoiceState>, new: serenity::VoiceState) {
        let mut tracked = self.tracker.lock().await;

        // only act on users we're actually watching
        if !tracked.contains_key(&new.user_id) {
            return;
        }

        let left_channel = old.as_ref().and_then(|vs| vs.channel_id).filter(|old_ch| new.channel_id.as_ref() != Some(old_ch));

        if let Some(old_channel_id) = left_channel {
            println!("[diag] tracked user {:?} left {:?}", new.user_id, old_channel_id);

            let Some(guild_id) = new.guild_id else {
                tracked.remove(&new.user_id);
                return;
            };

            // end spirc
            if let Some(spirc) = self.spirc_tracker.lock().await.remove(&guild_id) {
                if let Err(e) = spirc.shutdown() {
                    eprintln!("[diag] error shutting down spirc: {e:?}");
                }
            }

            // end voice connection
            let session = songbird::get(&ctx).await
                .expect("Voice session expected")
                .clone();
            if let Err(e) = session.remove(guild_id).await {
                eprintln!("[diag] error leaving voice channel: {e:?}");
            }

            // remove from tracking list
            tracked.remove(&new.user_id);
        }
    }
} 

// poise command types
struct Data {
    client_id: String,
    ah: auth::AuthHandler,
    redirect_uri: String,
    spirc_sessions: SpircSessions,
    active_users: ActiveUsers
} // User data, stored and accessible in all command invocations
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

pub async fn run_service(discord_token: String, client_id: String, auth: auth::AuthHandler, redirect_uri: String, cancel_token: CancellationToken) {
    let intents = serenity::GatewayIntents::non_privileged() | serenity::GatewayIntents::MESSAGE_CONTENT;

    let active_users: ActiveUsers =  Arc::new(Mutex::new(HashMap::new()));
    let tracker_handle = active_users.clone();

    let spirc_sessions: SpircSessions =  Arc::new(Mutex::new(HashMap::new()));
    let spirc_tracker = spirc_sessions.clone();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![speaker(), end_speaker()],
            ..Default::default()
        })
        // Found this in the poise docs. I think it's passing user data to the commands but damn it's weird
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data { client_id: client_id, ah: auth, redirect_uri: redirect_uri, spirc_sessions: spirc_sessions, active_users: active_users})
            })
        })
        .build();

    let mut client = serenity::ClientBuilder::new(discord_token, intents)
        .framework(framework)
        .event_handler(VoiceStateTracker { tracker: tracker_handle, spirc_tracker: spirc_tracker })
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

// todo: handle duplicate calls to channel
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

    if data.spirc_sessions.lock().await.contains_key(&guild_id) {
        return Err("Session already active".into());
    }

    // spotify auth
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
            data.active_users.lock().await.insert(ctx.author().id, channel_id);
        }
        Err(e) => {
            ctx.say(format!("Failed to join: {:?}", e)).await?;
            return Ok(());
        }
    }

    // check current contexts guild and creates a guard 
    if let Some(handler_lock) = sb_context.get(guild_id) {
        let mut handler = handler_lock.lock().await;

        // create the spirc session and stream object
        let (spirc, stream) = spotify::play_stream(token).await.unwrap();

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

    // stop/disconnect the Spirc device
    spirc.shutdown()?;
    // Leave the voice channel
    let session = songbird::get(ctx.serenity_context()).await
        .expect("Voice session expected")
        .clone();
    session.remove(guild_id).await?;
    ctx.say("Disconnected").await?;

    let _ = spirc;

    Ok(())
}