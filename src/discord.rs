use poise::serenity_prelude as serenity;
use rand::RngExt;
use songbird::SerenityInit;
use songbird::input::{Input};
use tokio_util::sync::CancellationToken;
use rand::{distr::Alphanumeric, Rng};


use crate::{auth};


// poise command types
struct Data {
    client_id: String,
    ah: auth::AuthHandler,
    redirect_uri: String
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
                Ok(Data { client_id: client_id, ah: auth, redirect_uri: redirect_uri})
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
        let handler = handler_lock.lock().await;


        //todo: setup librespot
        // the old version of this spawned an entire child process. I might try tokio spawn this time
        // let input = Input::from();
        // let _ = handler.play_input(input);

    } else {
        ctx.say("Error: Could not get voice handler after joining!").await?;
    }

    Ok(())
}

#[poise::command(slash_command)]
async fn end_speaker(
    ctx: Context<'_>
) -> Result<(), Error> {
    Ok(())
}